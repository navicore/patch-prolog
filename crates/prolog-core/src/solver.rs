use crate::builtins::{
    build_list, collect_list, exec_builtin, is_builtin, term_compare, BuiltinResult,
};
use crate::database::CompiledDatabase;
use crate::term::{Clause, Term, VarId};
use crate::unify::Substitution;
use fnv::FnvHashMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write as IoWrite;

/// Format a float for number_chars/number_codes, ensuring ".0" suffix for whole numbers.
fn format_float(f: f64) -> String {
    if f.is_nan() || f.is_infinite() {
        return format!("{}", f);
    }
    let s = format!("{}", f);
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{}.0", s)
    }
}

/// A solution: variable name -> resolved term.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub bindings: Vec<(String, Term)>,
}

/// Result of solving a query.
#[derive(Debug)]
pub enum SolveResult {
    /// Query succeeded with a solution.
    Success(Solution),
    /// No (more) solutions.
    Failure,
    /// A runtime error occurred.
    Error(String),
}

/// Choice point for backtracking.
struct ChoicePoint {
    /// Goal list at the time this choice was created.
    goals: VecDeque<Term>,
    /// Remaining untried clause indices.
    untried: Vec<usize>,
    /// Trail mark for undoing substitution bindings.
    trail_mark: usize,
    /// Variable counter at the time this choice was created.
    var_counter: VarId,
    /// Cut barrier: if true, backtracking past this point is blocked.
    cut_barrier: bool,
    /// Whether this is a disjunction choice point (alternative branch).
    disjunction: bool,
}

pub struct Solver<'a> {
    db: &'a CompiledDatabase,
    subst: Substitution,
    var_counter: VarId,
    query_vars: FnvHashMap<String, VarId>,
    choice_stack: Vec<ChoicePoint>,
    /// Limit on number of solutions to return.
    limit: Option<usize>,
    solutions_found: usize,
    /// Current number of goal resolution steps.
    steps: usize,
    /// Maximum allowed steps before returning an error.
    max_depth: usize,
    /// Mutable interner for runtime atom creation (atom_concat, atom_chars, etc.).
    interner: crate::term::StringInterner,
    /// Flag: cut was triggered inside try_solve_once, prevents clause alternatives.
    cut_in_try_solve: bool,
}

/// Find the maximum variable ID in a term.
fn max_var_in_term(term: &Term) -> Option<VarId> {
    match term {
        Term::Var(id) => Some(*id),
        Term::Atom(_) | Term::Integer(_) | Term::Float(_) => None,
        Term::Compound { args, .. } => args.iter().filter_map(max_var_in_term).max(),
        Term::List { head, tail } => {
            let h = max_var_in_term(head);
            let t = max_var_in_term(tail);
            h.max(t)
        }
    }
}

impl<'a> Solver<'a> {
    /// Create a new solver for a query against a compiled database.
    pub fn new(
        db: &'a CompiledDatabase,
        goals: Vec<Term>,
        query_vars: FnvHashMap<String, VarId>,
    ) -> Self {
        // Start var counter above all variable IDs in query (including anonymous _)
        let max_from_vars = query_vars.values().copied().max().unwrap_or(0);
        let max_from_goals = goals.iter().filter_map(max_var_in_term).max().unwrap_or(0);
        let initial_var_counter = max_from_vars.max(max_from_goals) + 1;
        let mut solver = Solver {
            interner: db.interner.clone(),
            db,
            subst: Substitution::new(),
            var_counter: initial_var_counter,
            query_vars,
            choice_stack: Vec::new(),
            limit: None,
            solutions_found: 0,
            steps: 0,
            max_depth: 10_000,
            cut_in_try_solve: false,
        };
        // Push the initial goal list as a choice point with no alternatives
        // (this is just to set up the initial state; the real solving starts in next())
        solver.choice_stack.push(ChoicePoint {
            goals: VecDeque::from(goals),
            untried: vec![],
            trail_mark: 0,
            var_counter: initial_var_counter,
            cut_barrier: false,
            disjunction: false,
        });
        solver
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Get the solver's interner (includes any runtime-created atoms).
    pub fn interner(&self) -> &crate::term::StringInterner {
        &self.interner
    }

    /// Get the next solution (or failure/error).
    pub fn next(&mut self) -> SolveResult {
        if let Some(limit) = self.limit {
            if self.solutions_found >= limit {
                return SolveResult::Failure;
            }
        }

        // If this is the first call, start from the initial choice point's goals
        if self.solutions_found == 0 && !self.choice_stack.is_empty() {
            let initial = self.choice_stack.pop().unwrap();
            self.var_counter = initial.var_counter;
            return self.solve(initial.goals);
        }

        // Otherwise, backtrack to find the next solution
        self.backtrack()
    }

    /// Enumerate all solutions.
    pub fn all_solutions(self) -> Result<Vec<Solution>, String> {
        self.all_solutions_with_interner().map(|(sols, _)| sols)
    }

    /// Enumerate all solutions, returning the interner for term display.
    pub fn all_solutions_with_interner(
        mut self,
    ) -> Result<(Vec<Solution>, crate::term::StringInterner), String> {
        let mut solutions = Vec::new();
        loop {
            match self.next() {
                SolveResult::Success(sol) => solutions.push(sol),
                SolveResult::Failure => return Ok((solutions, self.interner)),
                SolveResult::Error(e) => return Err(e),
            }
        }
    }

    /// Core solve loop: process goals one at a time.
    fn solve(&mut self, mut goals: VecDeque<Term>) -> SolveResult {
        loop {
            if goals.is_empty() {
                // Success! Extract the solution.
                self.solutions_found += 1;
                return SolveResult::Success(self.extract_solution());
            }

            self.steps += 1;
            if self.steps > self.max_depth {
                return SolveResult::Error(format!(
                    "Maximum step limit exceeded ({})",
                    self.max_depth
                ));
            }

            let goal = goals.pop_front().unwrap();
            let walked_goal = self.subst.walk(&goal);

            if is_builtin(&walked_goal, &self.interner) {
                match exec_builtin(&walked_goal, &mut self.subst, &self.interner) {
                    Ok(BuiltinResult::Success) => {
                        // Continue with remaining goals
                        continue;
                    }
                    Ok(BuiltinResult::Failure) => {
                        return self.backtrack();
                    }
                    Ok(BuiltinResult::Cut) => {
                        // Remove all choice points up to and including the nearest cut barrier
                        while let Some(cp) = self.choice_stack.pop() {
                            if cp.cut_barrier {
                                break;
                            }
                        }
                        // Continue with remaining goals
                        continue;
                    }
                    Ok(BuiltinResult::NegationAsFailure(inner_goal)) => {
                        // Try to solve the inner goal
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;
                        let saved_stack_len = self.choice_stack.len();

                        // Create a mini-solver for the inner goal
                        let inner_result = self.try_solve_once(vec![inner_goal]);

                        // Restore state regardless of outcome
                        self.subst.undo_to(mark);
                        self.var_counter = saved_counter;
                        self.choice_stack.truncate(saved_stack_len);

                        if inner_result {
                            // Inner goal succeeded, so \+ fails
                            return self.backtrack();
                        } else {
                            // Inner goal failed, so \+ succeeds
                            continue;
                        }
                    }
                    Ok(BuiltinResult::Disjunction(left, right)) => {
                        // Try left first; push choice point for right
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;

                        // Build the alternative goal list: right + remaining goals
                        let mut alt_goals = VecDeque::from(vec![right]);
                        alt_goals.extend(goals.iter().cloned());

                        // Push a disjunction choice point
                        self.choice_stack.push(ChoicePoint {
                            goals: alt_goals,
                            untried: vec![],
                            trail_mark: mark,
                            var_counter: saved_counter,
                            cut_barrier: false,
                            disjunction: true,
                        });

                        // Continue with left branch + remaining goals
                        goals.push_front(left);
                        continue;
                    }
                    Ok(BuiltinResult::IfThenElse(cond, then, else_branch)) => {
                        // Try cond; if succeeds, commit to then (cut alternatives)
                        // If cond fails, try else_branch
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;
                        let saved_stack_len = self.choice_stack.len();

                        if self.try_solve_once(vec![cond]) {
                            // Cond succeeded — keep bindings, truncate only choice stack
                            self.choice_stack.truncate(saved_stack_len);
                            goals.push_front(then);
                            continue;
                        } else {
                            // Cond failed — restore and try else
                            self.subst.undo_to(mark);
                            self.var_counter = saved_counter;
                            self.choice_stack.truncate(saved_stack_len);

                            goals.push_front(else_branch);
                            continue;
                        }
                    }
                    Ok(BuiltinResult::IfThen(cond, then)) => {
                        // Like if-then-else but no else (fails if cond fails)
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;
                        let saved_stack_len = self.choice_stack.len();

                        if self.try_solve_once(vec![cond]) {
                            // Cond succeeded — keep bindings, truncate only choice stack
                            self.choice_stack.truncate(saved_stack_len);
                            goals.push_front(then);
                            continue;
                        } else {
                            self.subst.undo_to(mark);
                            self.var_counter = saved_counter;
                            self.choice_stack.truncate(saved_stack_len);
                            return self.backtrack();
                        }
                    }
                    Ok(BuiltinResult::Conjunction(a, b)) => {
                        // Flatten conjunction into goal list
                        goals.push_front(b);
                        goals.push_front(a);
                        continue;
                    }
                    Ok(BuiltinResult::FindAll(template, goal, result_var)) => {
                        match self.exec_findall(template, goal) {
                            Ok(result_list) => {
                                if self.subst.unify(&result_var, &result_list) {
                                    continue;
                                } else {
                                    return self.backtrack();
                                }
                            }
                            Err(e) => return SolveResult::Error(e),
                        }
                    }
                    Ok(BuiltinResult::Once(inner_goal)) => {
                        // once/1: solve inner goal, keep bindings from first success,
                        // remove any choice points created by the inner goal.
                        let saved_stack_len = self.choice_stack.len();

                        // try_solve_once keeps bindings on success
                        if self.try_solve_once(vec![inner_goal]) {
                            // Truncate choice stack to remove inner choice points
                            self.choice_stack.truncate(saved_stack_len);
                            continue;
                        } else {
                            self.choice_stack.truncate(saved_stack_len);
                            return self.backtrack();
                        }
                    }
                    Ok(BuiltinResult::Call(inner_goal)) => {
                        // call/1: just execute the term as a goal
                        let walked = self.subst.walk(&inner_goal);
                        goals.push_front(walked);
                        continue;
                    }
                    Ok(BuiltinResult::AtomLength(atom_arg, len_arg)) => {
                        let walked = self.subst.walk(&atom_arg);
                        if let Term::Atom(id) = walked {
                            let name_str = self.interner.resolve(id);
                            let len = name_str.chars().count() as i64;
                            if self.subst.unify(&len_arg, &Term::Integer(len)) {
                                continue;
                            } else {
                                return self.backtrack();
                            }
                        } else {
                            return SolveResult::Error(
                                "atom_length/2: first argument must be an atom".to_string(),
                            );
                        }
                    }
                    Ok(BuiltinResult::AtomConcat(a_arg, b_arg, result_arg)) => {
                        let a = self.subst.walk(&a_arg);
                        let b = self.subst.walk(&b_arg);
                        if let (Term::Atom(id_a), Term::Atom(id_b)) = (&a, &b) {
                            let s = format!(
                                "{}{}",
                                self.interner.resolve(*id_a),
                                self.interner.resolve(*id_b)
                            );
                            let result_id = self.interner.intern(&s);
                            if self.subst.unify(&result_arg, &Term::Atom(result_id)) {
                                continue;
                            } else {
                                return self.backtrack();
                            }
                        } else {
                            return SolveResult::Error(
                                "atom_concat/3: first two arguments must be atoms".to_string(),
                            );
                        }
                    }
                    Ok(BuiltinResult::AtomChars(atom_arg, list_arg)) => {
                        let walked = self.subst.walk(&atom_arg);
                        if let Term::Atom(id) = walked {
                            // Forward: atom -> char list
                            let name_str = self.interner.resolve(id).to_string();
                            let nil_id = self.interner.lookup("[]").expect("[] must be interned");
                            let mut list = Term::Atom(nil_id);
                            for ch in name_str.chars().rev() {
                                let ch_id = self.interner.intern(&ch.to_string());
                                list = Term::List {
                                    head: Box::new(Term::Atom(ch_id)),
                                    tail: Box::new(list),
                                };
                            }
                            if self.subst.unify(&list_arg, &list) {
                                continue;
                            } else {
                                return self.backtrack();
                            }
                        } else if let Term::Var(_) = walked {
                            // Reverse: char list -> atom
                            let wlist = self.subst.apply(&list_arg);
                            if let Some(elems) = collect_list(&wlist, &self.interner) {
                                let s: Option<String> = elems
                                    .iter()
                                    .map(|e| {
                                        if let Term::Atom(id) = e {
                                            let ch = self.interner.resolve(*id);
                                            if ch.chars().count() == 1 {
                                                Some(ch.to_string())
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if let Some(s) = s {
                                    let atom_id = self.interner.intern(&s);
                                    if self.subst.unify(&atom_arg, &Term::Atom(atom_id)) {
                                        continue;
                                    }
                                }
                                return self.backtrack();
                            }
                            return SolveResult::Error(
                                "atom_chars/2: second argument must be a character list"
                                    .to_string(),
                            );
                        } else {
                            return SolveResult::Error(
                                "atom_chars/2: first argument must be an atom or variable"
                                    .to_string(),
                            );
                        }
                    }
                    Ok(BuiltinResult::Write(term)) => {
                        let resolved = self.subst.apply(&term);
                        let s = term_to_string(&resolved, &self.interner);
                        print!("{}", s);
                        let _ = std::io::stdout().flush();
                        continue;
                    }
                    Ok(BuiltinResult::Writeln(term)) => {
                        let resolved = self.subst.apply(&term);
                        let s = term_to_string(&resolved, &self.interner);
                        println!("{}", s);
                        continue;
                    }
                    Ok(BuiltinResult::Nl) => {
                        println!();
                        continue;
                    }
                    Ok(BuiltinResult::Compare(order_arg, t1, t2)) => {
                        let w1 = self.subst.apply(&t1);
                        let w2 = self.subst.apply(&t2);
                        let cmp = term_compare(&w1, &w2, &self.interner);
                        let order_name = match cmp {
                            std::cmp::Ordering::Less => "<",
                            std::cmp::Ordering::Equal => "=",
                            std::cmp::Ordering::Greater => ">",
                        };
                        let order_id = self.interner.intern(order_name);
                        if self.subst.unify(&order_arg, &Term::Atom(order_id)) {
                            continue;
                        } else {
                            return self.backtrack();
                        }
                    }
                    Ok(BuiltinResult::Functor(term_arg, name_arg, arity_arg)) => {
                        let walked = self.subst.walk(&term_arg);
                        match &walked {
                            Term::Atom(id) => {
                                // functor(atom, atom, 0)
                                if self.subst.unify(&name_arg, &Term::Atom(*id))
                                    && self.subst.unify(&arity_arg, &Term::Integer(0))
                                {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Integer(_) | Term::Float(_) => {
                                if self.subst.unify(&name_arg, &walked)
                                    && self.subst.unify(&arity_arg, &Term::Integer(0))
                                {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Compound { functor, args } => {
                                if self.subst.unify(&name_arg, &Term::Atom(*functor))
                                    && self
                                        .subst
                                        .unify(&arity_arg, &Term::Integer(args.len() as i64))
                                {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::List { .. } => {
                                // Lists are ./2
                                let dot_id = self.interner.intern(".");
                                if self.subst.unify(&name_arg, &Term::Atom(dot_id))
                                    && self.subst.unify(&arity_arg, &Term::Integer(2))
                                {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Var(_) => {
                                // First arg is a variable — try to construct from name+arity
                                let wname = self.subst.walk(&name_arg);
                                let warity = self.subst.walk(&arity_arg);
                                if let (Term::Atom(name_id), Term::Integer(0)) = (&wname, &warity) {
                                    if self.subst.unify(&term_arg, &Term::Atom(*name_id)) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                if let (Term::Integer(_), Term::Integer(0)) = (&wname, &warity) {
                                    if self.subst.unify(&term_arg, &wname) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                if let (Term::Float(_), Term::Integer(0)) = (&wname, &warity) {
                                    if self.subst.unify(&term_arg, &wname) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                if let (Term::Atom(name_id), Term::Integer(arity)) =
                                    (&wname, &warity)
                                {
                                    if *arity < 0 {
                                        return SolveResult::Error(
                                            "functor/3: arity must be non-negative".to_string(),
                                        );
                                    }
                                    if *arity > 1024 {
                                        return SolveResult::Error(
                                            "functor/3: arity too large (max 1024)".to_string(),
                                        );
                                    }
                                    if *arity > 0 {
                                        let args: Vec<Term> = (0..*arity as u32)
                                            .map(|_| {
                                                let v = self.var_counter;
                                                self.var_counter += 1;
                                                Term::Var(v)
                                            })
                                            .collect();
                                        let constructed = Term::Compound {
                                            functor: *name_id,
                                            args,
                                        };
                                        if self.subst.unify(&term_arg, &constructed) {
                                            continue;
                                        }
                                        return self.backtrack();
                                    }
                                }
                                return SolveResult::Error(
                                    "functor/3: insufficient arguments".to_string(),
                                );
                            }
                        }
                    }
                    Ok(BuiltinResult::Arg(n_arg, term_arg, result_arg)) => {
                        let wn = self.subst.walk(&n_arg);
                        let wterm = self.subst.walk(&term_arg);
                        if let Term::Integer(n) = wn {
                            let args_list = match &wterm {
                                Term::Compound { args, .. } => Some(args.as_slice()),
                                Term::List { .. } => {
                                    // Treat as .(H,T) with 2 args — handled below
                                    None
                                }
                                _ => {
                                    return SolveResult::Error(
                                        "arg/3: second argument must be compound".to_string(),
                                    );
                                }
                            };
                            if let Some(args) = args_list {
                                if n >= 1 && (n as usize) <= args.len() {
                                    let arg = args[(n - 1) as usize].clone();
                                    if self.subst.unify(&result_arg, &arg) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                return self.backtrack();
                            }
                            // Handle list case: .(H,T)
                            if let Term::List { head, tail } = &wterm {
                                match n {
                                    1 => {
                                        if self.subst.unify(&result_arg, head) {
                                            continue;
                                        }
                                        return self.backtrack();
                                    }
                                    2 => {
                                        if self.subst.unify(&result_arg, tail) {
                                            continue;
                                        }
                                        return self.backtrack();
                                    }
                                    _ => return self.backtrack(),
                                }
                            }
                            return self.backtrack();
                        }
                        return SolveResult::Error(
                            "arg/3: first argument must be integer".to_string(),
                        );
                    }
                    Ok(BuiltinResult::Univ(term_arg, list_arg)) => {
                        let walked = self.subst.walk(&term_arg);
                        match &walked {
                            Term::Var(_) => {
                                // Construct term from list (apply to deeply resolve variables)
                                let wlist = self.subst.apply(&list_arg);
                                if let Some(elems) = collect_list(&wlist, &self.interner) {
                                    if elems.is_empty() {
                                        return SolveResult::Error(
                                            "=../2: list must not be empty".to_string(),
                                        );
                                    }
                                    if let Term::Atom(functor_id) = &elems[0] {
                                        if elems.len() == 1 {
                                            if self.subst.unify(&term_arg, &Term::Atom(*functor_id))
                                            {
                                                continue;
                                            }
                                        } else {
                                            let constructed = Term::Compound {
                                                functor: *functor_id,
                                                args: elems[1..].to_vec(),
                                            };
                                            if self.subst.unify(&term_arg, &constructed) {
                                                continue;
                                            }
                                        }
                                    } else if elems.len() == 1 {
                                        // number =.. [number]
                                        if self.subst.unify(&term_arg, &elems[0]) {
                                            continue;
                                        }
                                    } else {
                                        return SolveResult::Error(
                                            "=../2: functor must be an atom when arity > 0"
                                                .to_string(),
                                        );
                                    }
                                    return self.backtrack();
                                }
                                return SolveResult::Error(
                                    "=../2: second argument must be a list".to_string(),
                                );
                            }
                            Term::Atom(id) => {
                                let elems = vec![Term::Atom(*id)];
                                let list = build_list(elems, &self.interner);
                                if self.subst.unify(&list_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Integer(_) | Term::Float(_) => {
                                let elems = vec![walked.clone()];
                                let list = build_list(elems, &self.interner);
                                if self.subst.unify(&list_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Compound { functor, args } => {
                                let mut elems = vec![Term::Atom(*functor)];
                                elems.extend(args.clone());
                                let list = build_list(elems, &self.interner);
                                if self.subst.unify(&list_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::List { head, tail } => {
                                let dot_id = self.interner.intern(".");
                                let elems = vec![Term::Atom(dot_id), *head.clone(), *tail.clone()];
                                let list = build_list(elems, &self.interner);
                                if self.subst.unify(&list_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                        }
                    }
                    Ok(BuiltinResult::Between(low_arg, high_arg, x_arg)) => {
                        let wlow = self.subst.walk(&low_arg);
                        let whigh = self.subst.walk(&high_arg);
                        if let (Term::Integer(low), Term::Integer(high)) = (&wlow, &whigh) {
                            if low > high {
                                return self.backtrack();
                            }
                            let mark = self.subst.trail_mark();
                            let saved_counter = self.var_counter;
                            // Push choice points for low+1..=high
                            if *low < *high {
                                let new_low = Term::Integer(match low.checked_add(1) {
                                    Some(v) => v,
                                    None => {
                                        return SolveResult::Error(
                                            "between/3: integer overflow".to_string(),
                                        )
                                    }
                                });
                                let between_functor = self.interner.intern("between");
                                let alt_goal = Term::Compound {
                                    functor: between_functor,
                                    args: vec![new_low, whigh.clone(), x_arg.clone()],
                                };
                                let mut alt_goals = VecDeque::from(vec![alt_goal]);
                                alt_goals.extend(goals.iter().cloned());
                                self.choice_stack.push(ChoicePoint {
                                    goals: alt_goals,
                                    untried: vec![],
                                    trail_mark: mark,
                                    var_counter: saved_counter,
                                    cut_barrier: false,
                                    disjunction: true,
                                });
                            }
                            if self.subst.unify(&x_arg, &Term::Integer(*low)) {
                                continue;
                            }
                            return self.backtrack();
                        }
                        return SolveResult::Error(
                            "between/3: first two arguments must be integers".to_string(),
                        );
                    }
                    Ok(BuiltinResult::CopyTerm(original, copy)) => {
                        let walked = self.subst.walk(&original);
                        let copied = self.copy_term_fresh(&walked);
                        if self.subst.unify(&copy, &copied) {
                            continue;
                        }
                        return self.backtrack();
                    }
                    Ok(BuiltinResult::Succ(x_arg, s_arg)) => {
                        let wx = self.subst.walk(&x_arg);
                        let ws = self.subst.walk(&s_arg);
                        match (&wx, &ws) {
                            (Term::Integer(x), _) if *x >= 0 => match x.checked_add(1) {
                                Some(result) => {
                                    if self.subst.unify(&s_arg, &Term::Integer(result)) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                None => {
                                    return SolveResult::Error(
                                        "succ/2: integer overflow".to_string(),
                                    )
                                }
                            },
                            (_, Term::Integer(s)) if *s > 0 => {
                                if self.subst.unify(&x_arg, &Term::Integer(s - 1)) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            (_, Term::Integer(s)) if *s == 0 => {
                                // succ(X, 0): no non-negative predecessor of 0, fail
                                return self.backtrack();
                            }
                            (Term::Integer(_), _) => {
                                return SolveResult::Error(
                                    "succ/2: argument must be non-negative".to_string(),
                                );
                            }
                            (_, Term::Integer(_)) => {
                                // s < 0: succ is only defined for natural numbers
                                return SolveResult::Error(
                                    "succ/2: successor must be non-negative".to_string(),
                                );
                            }
                            _ => {
                                return SolveResult::Error(
                                    "succ/2: at least one argument must be an integer".to_string(),
                                );
                            }
                        }
                    }
                    Ok(BuiltinResult::Plus(x_arg, y_arg, z_arg)) => {
                        let wx = self.subst.walk(&x_arg);
                        let wy = self.subst.walk(&y_arg);
                        let wz = self.subst.walk(&z_arg);
                        match (&wx, &wy, &wz) {
                            (Term::Integer(x), Term::Integer(y), _) => match x.checked_add(*y) {
                                Some(result) => {
                                    if self.subst.unify(&z_arg, &Term::Integer(result)) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                None => {
                                    return SolveResult::Error(
                                        "plus/3: integer overflow".to_string(),
                                    )
                                }
                            },
                            (Term::Integer(x), _, Term::Integer(z)) => match z.checked_sub(*x) {
                                Some(result) => {
                                    if self.subst.unify(&y_arg, &Term::Integer(result)) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                None => {
                                    return SolveResult::Error(
                                        "plus/3: integer overflow".to_string(),
                                    )
                                }
                            },
                            (_, Term::Integer(y), Term::Integer(z)) => match z.checked_sub(*y) {
                                Some(result) => {
                                    if self.subst.unify(&x_arg, &Term::Integer(result)) {
                                        continue;
                                    }
                                    return self.backtrack();
                                }
                                None => {
                                    return SolveResult::Error(
                                        "plus/3: integer overflow".to_string(),
                                    )
                                }
                            },
                            _ => {
                                return SolveResult::Error(
                                    "plus/3: at least two arguments must be integers".to_string(),
                                );
                            }
                        }
                    }
                    Ok(BuiltinResult::MSort(list_arg, sorted_arg)) => {
                        let wlist = self.subst.apply(&list_arg);
                        if let Some(mut elems) = collect_list(&wlist, &self.interner) {
                            elems.sort_by(|a, b| term_compare(a, b, &self.interner));
                            let sorted = build_list(elems, &self.interner);
                            if self.subst.unify(&sorted_arg, &sorted) {
                                continue;
                            }
                            return self.backtrack();
                        }
                        return SolveResult::Error(
                            "msort/2: first argument must be a list".to_string(),
                        );
                    }
                    Ok(BuiltinResult::Sort(list_arg, sorted_arg)) => {
                        let wlist = self.subst.apply(&list_arg);
                        if let Some(mut elems) = collect_list(&wlist, &self.interner) {
                            elems.sort_by(|a, b| term_compare(a, b, &self.interner));
                            elems.dedup_by(|a, b| {
                                term_compare(a, b, &self.interner) == std::cmp::Ordering::Equal
                            });
                            let sorted = build_list(elems, &self.interner);
                            if self.subst.unify(&sorted_arg, &sorted) {
                                continue;
                            }
                            return self.backtrack();
                        }
                        return SolveResult::Error(
                            "sort/2: first argument must be a list".to_string(),
                        );
                    }
                    Ok(BuiltinResult::NumberChars(num_arg, chars_arg)) => {
                        let wnum = self.subst.walk(&num_arg);
                        match &wnum {
                            Term::Integer(n) => {
                                let s = n.to_string();
                                let nil_id =
                                    self.interner.lookup("[]").expect("[] must be interned");
                                let mut list = Term::Atom(nil_id);
                                for ch in s.chars().rev() {
                                    let ch_id = self.interner.intern(&ch.to_string());
                                    list = Term::List {
                                        head: Box::new(Term::Atom(ch_id)),
                                        tail: Box::new(list),
                                    };
                                }
                                if self.subst.unify(&chars_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Float(f) => {
                                let s = format_float(*f);
                                let nil_id =
                                    self.interner.lookup("[]").expect("[] must be interned");
                                let mut list = Term::Atom(nil_id);
                                for ch in s.chars().rev() {
                                    let ch_id = self.interner.intern(&ch.to_string());
                                    list = Term::List {
                                        head: Box::new(Term::Atom(ch_id)),
                                        tail: Box::new(list),
                                    };
                                }
                                if self.subst.unify(&chars_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Var(_) => {
                                // Try to parse chars back to number
                                let wchars = self.subst.apply(&chars_arg);
                                if let Some(elems) = collect_list(&wchars, &self.interner) {
                                    let s: Option<String> = elems
                                        .iter()
                                        .map(|e| match e {
                                            Term::Atom(id) => {
                                                let ch = self.interner.resolve(*id);
                                                if ch.chars().count() == 1 {
                                                    Some(ch.to_string())
                                                } else {
                                                    None
                                                }
                                            }
                                            _ => None,
                                        })
                                        .collect();
                                    match s {
                                        Some(s) => {
                                            if let Ok(n) = s.parse::<i64>() {
                                                if self.subst.unify(&num_arg, &Term::Integer(n)) {
                                                    continue;
                                                }
                                            } else if let Ok(f) = s.parse::<f64>() {
                                                if self.subst.unify(&num_arg, &Term::Float(f)) {
                                                    continue;
                                                }
                                            }
                                            return self.backtrack();
                                        }
                                        None => {
                                            return SolveResult::Error(
                                                "number_chars/2: list elements must be single-character atoms".to_string(),
                                            );
                                        }
                                    }
                                }
                                return SolveResult::Error(
                                    "number_chars/2: at least one argument must be bound"
                                        .to_string(),
                                );
                            }
                            _ => {
                                return SolveResult::Error(
                                    "number_chars/2: first argument must be a number".to_string(),
                                );
                            }
                        }
                    }
                    Ok(BuiltinResult::NumberCodes(num_arg, codes_arg)) => {
                        let wnum = self.subst.walk(&num_arg);
                        match &wnum {
                            Term::Integer(n) => {
                                let s = n.to_string();
                                let nil_id =
                                    self.interner.lookup("[]").expect("[] must be interned");
                                let mut list = Term::Atom(nil_id);
                                for ch in s.chars().rev() {
                                    list = Term::List {
                                        head: Box::new(Term::Integer(ch as i64)),
                                        tail: Box::new(list),
                                    };
                                }
                                if self.subst.unify(&codes_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Float(f) => {
                                let s = format_float(*f);
                                let nil_id =
                                    self.interner.lookup("[]").expect("[] must be interned");
                                let mut list = Term::Atom(nil_id);
                                for ch in s.chars().rev() {
                                    list = Term::List {
                                        head: Box::new(Term::Integer(ch as i64)),
                                        tail: Box::new(list),
                                    };
                                }
                                if self.subst.unify(&codes_arg, &list) {
                                    continue;
                                }
                                return self.backtrack();
                            }
                            Term::Var(_) => {
                                // Try to parse codes back to number
                                let wcodes = self.subst.apply(&codes_arg);
                                if let Some(elems) = collect_list(&wcodes, &self.interner) {
                                    let s: Option<String> = elems
                                        .iter()
                                        .map(|e| {
                                            if let Term::Integer(code) = e {
                                                if *code >= 0 && *code <= 0x10FFFF {
                                                    char::from_u32(*code as u32)
                                                        .map(|c| c.to_string())
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    match s {
                                        Some(s) => {
                                            if let Ok(n) = s.parse::<i64>() {
                                                if self.subst.unify(&num_arg, &Term::Integer(n)) {
                                                    continue;
                                                }
                                            } else if let Ok(f) = s.parse::<f64>() {
                                                if self.subst.unify(&num_arg, &Term::Float(f)) {
                                                    continue;
                                                }
                                            }
                                            return self.backtrack();
                                        }
                                        None => {
                                            return SolveResult::Error(
                                                "number_codes/2: list elements must be valid character codes".to_string(),
                                            );
                                        }
                                    }
                                }
                                return SolveResult::Error(
                                    "number_codes/2: at least one argument must be bound"
                                        .to_string(),
                                );
                            }
                            _ => {
                                return SolveResult::Error(
                                    "number_codes/2: first argument must be a number".to_string(),
                                );
                            }
                        }
                    }
                    Err(e) => return SolveResult::Error(e),
                }
            } else {
                // User-defined predicate: look up candidate clauses
                let candidates = self.db.lookup(&walked_goal);
                if candidates.is_empty() {
                    return self.backtrack();
                }

                match self.try_clauses(walked_goal, goals, candidates) {
                    Some(new_goals) => {
                        goals = new_goals;
                        continue;
                    }
                    None => return self.backtrack(),
                }
            }
        }
    }

    /// Try candidate clauses for a goal. Returns new goal list if one succeeds.
    fn try_clauses(
        &mut self,
        goal: Term,
        rest_goals: VecDeque<Term>,
        candidates: Vec<usize>,
    ) -> Option<VecDeque<Term>> {
        for (i, &clause_idx) in candidates.iter().enumerate() {
            let mark = self.subst.trail_mark();
            let saved_counter = self.var_counter;

            let clause = &self.db.clauses[clause_idx];
            let renamed = self.rename_clause(clause);

            if self.subst.unify(&goal, &renamed.head) {
                // Build new goals: body of matched clause + remaining goals
                let mut new_goals: VecDeque<Term> = VecDeque::from(renamed.body);
                new_goals.extend(rest_goals.iter().cloned());

                // If there are more candidates, push a choice point
                if i + 1 < candidates.len() {
                    self.choice_stack.push(ChoicePoint {
                        goals: {
                            let mut g = VecDeque::from(vec![goal.clone()]);
                            g.extend(rest_goals);
                            g
                        },
                        untried: candidates[i + 1..].to_vec(),
                        trail_mark: mark,
                        var_counter: saved_counter,
                        cut_barrier: true,
                        disjunction: false,
                    });
                }

                return Some(new_goals);
            } else {
                // Unification failed; undo and try next clause
                self.subst.undo_to(mark);
                self.var_counter = saved_counter;
            }
        }
        None
    }

    /// Backtrack: pop choice points until we find one with alternatives.
    fn backtrack(&mut self) -> SolveResult {
        while let Some(cp) = self.choice_stack.pop() {
            // Restore state to this choice point
            self.subst.undo_to(cp.trail_mark);
            self.var_counter = cp.var_counter;

            if cp.disjunction {
                // Disjunction choice point: try the alternative branch directly
                return self.solve(cp.goals);
            }

            if cp.untried.is_empty() {
                // No alternatives at this level — keep backtracking
                continue;
            }

            // The first goal in cp.goals is the one we need to retry
            let mut cp_goals = cp.goals;
            let goal = cp_goals.pop_front().unwrap();
            let rest_goals = cp_goals;
            let candidates = cp.untried;

            match self.try_clauses(goal, rest_goals, candidates) {
                Some(new_goals) => {
                    return self.solve(new_goals);
                }
                None => {
                    // All remaining candidates failed — keep backtracking
                    continue;
                }
            }
        }
        SolveResult::Failure
    }

    /// Try to solve goals (used for negation-as-failure check).
    /// Returns true if the goals succeed at least once.
    fn try_solve_once(&mut self, goals: Vec<Term>) -> bool {
        let mut goal_list = VecDeque::from(goals);
        loop {
            if goal_list.is_empty() {
                return true;
            }

            self.steps += 1;
            if self.steps > self.max_depth {
                return false;
            }

            let goal = goal_list.pop_front().unwrap();
            let walked_goal = self.subst.walk(&goal);

            if is_builtin(&walked_goal, &self.interner) {
                match exec_builtin(&walked_goal, &mut self.subst, &self.interner) {
                    Ok(BuiltinResult::Success) => continue,
                    Ok(BuiltinResult::Failure) => return false,
                    Ok(BuiltinResult::Cut) => {
                        self.cut_in_try_solve = true;
                        continue;
                    }
                    Ok(BuiltinResult::NegationAsFailure(inner)) => {
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;
                        let inner_result = self.try_solve_once(vec![inner]);
                        self.subst.undo_to(mark);
                        self.var_counter = saved_counter;
                        if inner_result {
                            return false;
                        }
                        continue;
                    }
                    Ok(BuiltinResult::Conjunction(a, b)) => {
                        goal_list.push_front(b);
                        goal_list.push_front(a);
                        continue;
                    }
                    Ok(BuiltinResult::Disjunction(left, right)) => {
                        // Try left first
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;
                        let mut left_goals = vec![left];
                        left_goals.extend(goal_list.iter().cloned());
                        if self.try_solve_once(left_goals) {
                            return true;
                        }
                        self.subst.undo_to(mark);
                        self.var_counter = saved_counter;
                        // Try right
                        goal_list.push_front(right);
                        continue;
                    }
                    Ok(BuiltinResult::IfThenElse(cond, then, else_branch)) => {
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;
                        if self.try_solve_once(vec![cond]) {
                            // Keep bindings from cond
                            goal_list.push_front(then);
                            continue;
                        } else {
                            self.subst.undo_to(mark);
                            self.var_counter = saved_counter;
                            goal_list.push_front(else_branch);
                            continue;
                        }
                    }
                    Ok(BuiltinResult::IfThen(cond, then)) => {
                        let mark = self.subst.trail_mark();
                        let saved_counter = self.var_counter;
                        if self.try_solve_once(vec![cond]) {
                            // Keep bindings from cond
                            goal_list.push_front(then);
                            continue;
                        } else {
                            self.subst.undo_to(mark);
                            self.var_counter = saved_counter;
                            return false;
                        }
                    }
                    Ok(BuiltinResult::FindAll(template, goal, result_var)) => {
                        match self.exec_findall(template, goal) {
                            Ok(result_list) => {
                                if self.subst.unify(&result_var, &result_list) {
                                    continue;
                                } else {
                                    return false;
                                }
                            }
                            Err(_) => return false,
                        }
                    }
                    Ok(BuiltinResult::Once(inner_goal)) => {
                        // once/1 in try_solve_once: just solve the inner goal once
                        let walked = self.subst.walk(&inner_goal);
                        goal_list.push_front(walked);
                        continue;
                    }
                    Ok(BuiltinResult::Call(inner_goal)) => {
                        let walked = self.subst.walk(&inner_goal);
                        goal_list.push_front(walked);
                        continue;
                    }
                    Ok(BuiltinResult::AtomLength(atom_arg, len_arg)) => {
                        let walked = self.subst.walk(&atom_arg);
                        if let Term::Atom(id) = walked {
                            let len = self.interner.resolve(id).chars().count() as i64;
                            if self.subst.unify(&len_arg, &Term::Integer(len)) {
                                continue;
                            }
                        }
                        return false;
                    }
                    Ok(BuiltinResult::AtomConcat(a_arg, b_arg, result_arg)) => {
                        let a = self.subst.walk(&a_arg);
                        let b = self.subst.walk(&b_arg);
                        if let (Term::Atom(id_a), Term::Atom(id_b)) = (&a, &b) {
                            let s = format!(
                                "{}{}",
                                self.interner.resolve(*id_a),
                                self.interner.resolve(*id_b)
                            );
                            let result_id = self.interner.intern(&s);
                            if self.subst.unify(&result_arg, &Term::Atom(result_id)) {
                                continue;
                            }
                        }
                        return false;
                    }
                    Ok(BuiltinResult::AtomChars(atom_arg, list_arg)) => {
                        let walked = self.subst.walk(&atom_arg);
                        if let Term::Atom(id) = walked {
                            let name_str = self.interner.resolve(id).to_string();
                            let nil_id = self.interner.lookup("[]").expect("[] must be interned");
                            let mut list = Term::Atom(nil_id);
                            for ch in name_str.chars().rev() {
                                let ch_id = self.interner.intern(&ch.to_string());
                                list = Term::List {
                                    head: Box::new(Term::Atom(ch_id)),
                                    tail: Box::new(list),
                                };
                            }
                            if self.subst.unify(&list_arg, &list) {
                                continue;
                            }
                        } else if let Term::Var(_) = walked {
                            let wlist = self.subst.apply(&list_arg);
                            if let Some(elems) = collect_list(&wlist, &self.interner) {
                                let s: Option<String> = elems
                                    .iter()
                                    .map(|e| {
                                        if let Term::Atom(id) = e {
                                            let ch = self.interner.resolve(*id);
                                            if ch.chars().count() == 1 {
                                                Some(ch.to_string())
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if let Some(s) = s {
                                    let atom_id = self.interner.intern(&s);
                                    if self.subst.unify(&atom_arg, &Term::Atom(atom_id)) {
                                        continue;
                                    }
                                }
                            }
                        }
                        return false;
                    }
                    Ok(BuiltinResult::Between(low_arg, high_arg, x_arg)) => {
                        // between/3 needs explicit handling to backtrack through values
                        // in conjunctions like \+ (between(1,5,X), X > 3)
                        let wlow = self.subst.walk(&low_arg);
                        let whigh = self.subst.walk(&high_arg);
                        if let (Term::Integer(low), Term::Integer(high)) = (&wlow, &whigh) {
                            // If X is already bound, just check low <= X <= high (O(1))
                            let wx = self.subst.walk(&x_arg);
                            if let Term::Integer(x_val) = &wx {
                                if *x_val >= *low && *x_val <= *high {
                                    let remaining: Vec<Term> = goal_list.iter().cloned().collect();
                                    return self.try_solve_once(remaining);
                                }
                                return false;
                            }
                            // X is unbound: iterate (step counter limits runaway)
                            for val in *low..=*high {
                                let mark = self.subst.trail_mark();
                                let saved_counter = self.var_counter;
                                if self.subst.unify(&x_arg, &Term::Integer(val)) {
                                    let remaining: Vec<Term> = goal_list.iter().cloned().collect();
                                    if self.try_solve_once(remaining) {
                                        return true;
                                    }
                                }
                                self.subst.undo_to(mark);
                                self.var_counter = saved_counter;
                            }
                            return false;
                        }
                        return false;
                    }
                    Ok(other) => {
                        // Handle remaining builtins via try_exec_misc
                        if let Some(success) = self.try_exec_misc(other, &mut goal_list) {
                            if success {
                                continue;
                            } else {
                                return false;
                            }
                        }
                        return false;
                    }
                    Err(_) => return false,
                }
            }

            let candidates = self.db.lookup(&walked_goal);
            for &clause_idx in &candidates {
                let mark = self.subst.trail_mark();
                let saved_counter = self.var_counter;

                let clause = &self.db.clauses[clause_idx];
                let renamed = self.rename_clause(clause);

                if self.subst.unify(&walked_goal, &renamed.head) {
                    let mut new_goals = renamed.body;
                    new_goals.extend(goal_list.clone());
                    let saved_cut = self.cut_in_try_solve;
                    self.cut_in_try_solve = false;
                    if self.try_solve_once(new_goals) {
                        self.cut_in_try_solve = saved_cut;
                        return true;
                    }
                    if self.cut_in_try_solve {
                        // Cut was triggered — don't try more clauses
                        self.cut_in_try_solve = saved_cut;
                        self.subst.undo_to(mark);
                        self.var_counter = saved_counter;
                        return false;
                    }
                    self.cut_in_try_solve = saved_cut;
                }
                self.subst.undo_to(mark);
                self.var_counter = saved_counter;
            }
            return false;
        }
    }

    /// Execute findall/3: collect all instances of Template for which Goal succeeds.
    fn exec_findall(&mut self, template: Term, goal: Term) -> Result<Term, String> {
        let mark = self.subst.trail_mark();
        let saved_counter = self.var_counter;
        let saved_stack_len = self.choice_stack.len();

        let mut collected = Vec::new();
        self.try_solve_collecting(VecDeque::from(vec![goal]), &template, &mut collected);

        // Restore state
        self.subst.undo_to(mark);
        self.var_counter = saved_counter;
        self.choice_stack.truncate(saved_stack_len);

        // Build the result list from collected terms
        let nil_id = self.interner.lookup("[]").expect("[] must be interned");
        let mut result = Term::Atom(nil_id);
        for term in collected.into_iter().rev() {
            result = Term::List {
                head: Box::new(term),
                tail: Box::new(result),
            };
        }

        Ok(result)
    }

    /// Try to solve a list of goals, collecting template instances for each success.
    fn try_solve_collecting(
        &mut self,
        goals: VecDeque<Term>,
        template: &Term,
        results: &mut Vec<Term>,
    ) -> bool {
        if goals.is_empty() {
            results.push(self.subst.apply(template));
            return true;
        }

        self.steps += 1;
        if self.steps > self.max_depth {
            return false;
        }

        let mut goal_list = goals;
        let goal = goal_list.pop_front().unwrap();
        let walked_goal = self.subst.walk(&goal);

        if is_builtin(&walked_goal, &self.interner) {
            match exec_builtin(&walked_goal, &mut self.subst, &self.interner) {
                Ok(BuiltinResult::Success) => {
                    return self.try_solve_collecting(goal_list, template, results);
                }
                Ok(BuiltinResult::Cut) => {
                    return self.try_solve_collecting(goal_list, template, results);
                }
                Ok(BuiltinResult::Conjunction(a, b)) => {
                    goal_list.push_front(b);
                    goal_list.push_front(a);
                    return self.try_solve_collecting(goal_list, template, results);
                }
                Ok(BuiltinResult::NegationAsFailure(inner)) => {
                    let mark = self.subst.trail_mark();
                    let saved_counter = self.var_counter;
                    let inner_result = self.try_solve_once(vec![inner]);
                    self.subst.undo_to(mark);
                    self.var_counter = saved_counter;
                    if inner_result {
                        return false;
                    }
                    return self.try_solve_collecting(goal_list, template, results);
                }
                Ok(BuiltinResult::Disjunction(left, right)) => {
                    // Try left branch
                    let mark = self.subst.trail_mark();
                    let saved_counter = self.var_counter;
                    let mut left_goals = VecDeque::from(vec![left]);
                    left_goals.extend(goal_list.iter().cloned());
                    let found_left = self.try_solve_collecting(left_goals, template, results);
                    self.subst.undo_to(mark);
                    self.var_counter = saved_counter;
                    // Try right branch
                    let mut right_goals = VecDeque::from(vec![right]);
                    right_goals.extend(goal_list);
                    let found_right = self.try_solve_collecting(right_goals, template, results);
                    return found_left || found_right;
                }
                Ok(BuiltinResult::IfThenElse(cond, then, else_branch)) => {
                    let mark = self.subst.trail_mark();
                    let saved_counter = self.var_counter;
                    if self.try_solve_once(vec![cond]) {
                        // Keep bindings from cond
                        goal_list.push_front(then);
                        return self.try_solve_collecting(goal_list, template, results);
                    } else {
                        self.subst.undo_to(mark);
                        self.var_counter = saved_counter;
                        goal_list.push_front(else_branch);
                        return self.try_solve_collecting(goal_list, template, results);
                    }
                }
                Ok(BuiltinResult::IfThen(cond, then)) => {
                    let mark = self.subst.trail_mark();
                    let saved_counter = self.var_counter;
                    if self.try_solve_once(vec![cond]) {
                        // Keep bindings from cond
                        goal_list.push_front(then);
                        return self.try_solve_collecting(goal_list, template, results);
                    } else {
                        self.subst.undo_to(mark);
                        self.var_counter = saved_counter;
                        return false;
                    }
                }
                Ok(BuiltinResult::FindAll(tmpl, inner_goal, result_var)) => {
                    match self.exec_findall(tmpl, inner_goal) {
                        Ok(result_list) => {
                            if self.subst.unify(&result_var, &result_list) {
                                return self.try_solve_collecting(goal_list, template, results);
                            }
                            return false;
                        }
                        Err(_) => return false,
                    }
                }
                Ok(BuiltinResult::Once(inner_goal)) => {
                    let walked = self.subst.walk(&inner_goal);
                    if self.try_solve_once(vec![walked]) {
                        return self.try_solve_collecting(goal_list, template, results);
                    }
                    return false;
                }
                Ok(BuiltinResult::Call(inner_goal)) => {
                    let walked = self.subst.walk(&inner_goal);
                    goal_list.push_front(walked);
                    return self.try_solve_collecting(goal_list, template, results);
                }
                Ok(BuiltinResult::AtomLength(atom_arg, len_arg)) => {
                    let walked = self.subst.walk(&atom_arg);
                    if let Term::Atom(id) = walked {
                        let len = self.interner.resolve(id).chars().count() as i64;
                        if self.subst.unify(&len_arg, &Term::Integer(len)) {
                            return self.try_solve_collecting(goal_list, template, results);
                        }
                    }
                    return false;
                }
                Ok(BuiltinResult::AtomConcat(a_arg, b_arg, result_arg)) => {
                    let a = self.subst.walk(&a_arg);
                    let b = self.subst.walk(&b_arg);
                    if let (Term::Atom(id_a), Term::Atom(id_b)) = (&a, &b) {
                        let s = format!(
                            "{}{}",
                            self.interner.resolve(*id_a),
                            self.interner.resolve(*id_b)
                        );
                        let result_id = self.interner.intern(&s);
                        if self.subst.unify(&result_arg, &Term::Atom(result_id)) {
                            return self.try_solve_collecting(goal_list, template, results);
                        }
                    }
                    return false;
                }
                Ok(BuiltinResult::AtomChars(atom_arg, list_arg)) => {
                    let walked = self.subst.walk(&atom_arg);
                    if let Term::Atom(id) = walked {
                        let name_str = self.interner.resolve(id).to_string();
                        let nil_id = self.interner.lookup("[]").expect("[] must be interned");
                        let mut list = Term::Atom(nil_id);
                        for ch in name_str.chars().rev() {
                            let ch_id = self.interner.intern(&ch.to_string());
                            list = Term::List {
                                head: Box::new(Term::Atom(ch_id)),
                                tail: Box::new(list),
                            };
                        }
                        if self.subst.unify(&list_arg, &list) {
                            return self.try_solve_collecting(goal_list, template, results);
                        }
                    } else if let Term::Var(_) = walked {
                        let wlist = self.subst.apply(&list_arg);
                        if let Some(elems) = collect_list(&wlist, &self.interner) {
                            let s: Option<String> = elems
                                .iter()
                                .map(|e| {
                                    if let Term::Atom(id) = e {
                                        let ch = self.interner.resolve(*id);
                                        if ch.chars().count() == 1 {
                                            Some(ch.to_string())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if let Some(s) = s {
                                let atom_id = self.interner.intern(&s);
                                if self.subst.unify(&atom_arg, &Term::Atom(atom_id)) {
                                    return self.try_solve_collecting(goal_list, template, results);
                                }
                            }
                        }
                    }
                    return false;
                }
                Ok(BuiltinResult::Failure) => return false,
                Ok(BuiltinResult::Between(low_arg, high_arg, x_arg)) => {
                    // between/3 needs special handling to enumerate all values
                    let wlow = self.subst.walk(&low_arg);
                    let whigh = self.subst.walk(&high_arg);
                    if let (Term::Integer(low), Term::Integer(high)) = (&wlow, &whigh) {
                        // If X is already bound, just check low <= X <= high (O(1))
                        let wx = self.subst.walk(&x_arg);
                        if let Term::Integer(x_val) = &wx {
                            if *x_val >= *low && *x_val <= *high {
                                let remaining = goal_list.clone();
                                return self.try_solve_collecting(remaining, template, results);
                            }
                            return false;
                        }
                        // X is unbound: iterate (step counter limits runaway)
                        let mut found_any = false;
                        for val in *low..=*high {
                            let mark = self.subst.trail_mark();
                            let saved_counter = self.var_counter;
                            if self.subst.unify(&x_arg, &Term::Integer(val)) {
                                let remaining = goal_list.clone();
                                if self.try_solve_collecting(remaining, template, results) {
                                    found_any = true;
                                }
                            }
                            self.subst.undo_to(mark);
                            self.var_counter = saved_counter;
                        }
                        return found_any;
                    }
                    return false;
                }
                Ok(other) => {
                    // Handle remaining builtins via try_exec_misc
                    if let Some(success) = self.try_exec_misc(other, &mut goal_list) {
                        if success {
                            return self.try_solve_collecting(goal_list, template, results);
                        }
                    }
                    return false;
                }
                Err(_) => return false,
            }
        }

        let candidates = self.db.lookup(&walked_goal);
        let mut found_any = false;
        for &clause_idx in &candidates {
            let mark = self.subst.trail_mark();
            let saved_counter = self.var_counter;

            let clause = &self.db.clauses[clause_idx];
            let renamed = self.rename_clause(clause);

            if self.subst.unify(&walked_goal, &renamed.head) {
                let mut new_goals: VecDeque<Term> = VecDeque::from(renamed.body);
                new_goals.extend(goal_list.iter().cloned());
                if self.try_solve_collecting(new_goals, template, results) {
                    found_any = true;
                }
            }
            self.subst.undo_to(mark);
            self.var_counter = saved_counter;
        }
        found_any
    }

    /// Rename all variables in a clause to fresh IDs to avoid collisions.
    fn rename_clause(&mut self, clause: &Clause) -> Clause {
        let mut var_map: FnvHashMap<VarId, VarId> = FnvHashMap::default();
        Clause {
            head: self.rename_term(&clause.head, &mut var_map),
            body: clause
                .body
                .iter()
                .map(|t| self.rename_term(t, &mut var_map))
                .collect(),
        }
    }

    fn rename_term(&mut self, term: &Term, var_map: &mut FnvHashMap<VarId, VarId>) -> Term {
        match term {
            Term::Var(id) => {
                let new_id = *var_map.entry(*id).or_insert_with(|| {
                    let fresh = self.var_counter;
                    self.var_counter += 1;
                    fresh
                });
                Term::Var(new_id)
            }
            Term::Compound { functor, args } => Term::Compound {
                functor: *functor,
                args: args.iter().map(|a| self.rename_term(a, var_map)).collect(),
            },
            Term::List { head, tail } => Term::List {
                head: Box::new(self.rename_term(head, var_map)),
                tail: Box::new(self.rename_term(tail, var_map)),
            },
            _ => term.clone(),
        }
    }

    /// Handle miscellaneous builtins shared between try_solve_once and try_solve_collecting.
    /// Returns Some(true) on success, Some(false) on failure, None on error (treat as failure).
    fn try_exec_misc(
        &mut self,
        result: BuiltinResult,
        _goal_list: &mut VecDeque<Term>,
    ) -> Option<bool> {
        match result {
            BuiltinResult::Write(term) => {
                let resolved = self.subst.apply(&term);
                print!("{}", term_to_string(&resolved, &self.interner));
                let _ = std::io::stdout().flush();
                Some(true)
            }
            BuiltinResult::Writeln(term) => {
                let resolved = self.subst.apply(&term);
                println!("{}", term_to_string(&resolved, &self.interner));
                Some(true)
            }
            BuiltinResult::Nl => {
                println!();
                Some(true)
            }
            BuiltinResult::Compare(order_arg, t1, t2) => {
                let w1 = self.subst.apply(&t1);
                let w2 = self.subst.apply(&t2);
                let cmp = term_compare(&w1, &w2, &self.interner);
                let order_name = match cmp {
                    std::cmp::Ordering::Less => "<",
                    std::cmp::Ordering::Equal => "=",
                    std::cmp::Ordering::Greater => ">",
                };
                let order_id = self.interner.intern(order_name);
                Some(self.subst.unify(&order_arg, &Term::Atom(order_id)))
            }
            BuiltinResult::Functor(term_arg, name_arg, arity_arg) => {
                let walked = self.subst.walk(&term_arg);
                match &walked {
                    Term::Atom(id) => Some(
                        self.subst.unify(&name_arg, &Term::Atom(*id))
                            && self.subst.unify(&arity_arg, &Term::Integer(0)),
                    ),
                    Term::Integer(_) | Term::Float(_) => Some(
                        self.subst.unify(&name_arg, &walked)
                            && self.subst.unify(&arity_arg, &Term::Integer(0)),
                    ),
                    Term::Compound { functor, args } => Some(
                        self.subst.unify(&name_arg, &Term::Atom(*functor))
                            && self
                                .subst
                                .unify(&arity_arg, &Term::Integer(args.len() as i64)),
                    ),
                    Term::List { .. } => {
                        let dot_id = self.interner.intern(".");
                        Some(
                            self.subst.unify(&name_arg, &Term::Atom(dot_id))
                                && self.subst.unify(&arity_arg, &Term::Integer(2)),
                        )
                    }
                    Term::Var(_) => {
                        let wname = self.subst.walk(&name_arg);
                        let warity = self.subst.walk(&arity_arg);
                        match (&wname, &warity) {
                            (Term::Atom(name_id), Term::Integer(0)) => {
                                Some(self.subst.unify(&term_arg, &Term::Atom(*name_id)))
                            }
                            (Term::Integer(_) | Term::Float(_), Term::Integer(0)) => {
                                Some(self.subst.unify(&term_arg, &wname))
                            }
                            (Term::Atom(name_id), Term::Integer(arity))
                                if *arity > 0 && *arity <= 1024 =>
                            {
                                let args: Vec<Term> = (0..*arity as u32)
                                    .map(|_| {
                                        let v = self.var_counter;
                                        self.var_counter += 1;
                                        Term::Var(v)
                                    })
                                    .collect();
                                let constructed = Term::Compound {
                                    functor: *name_id,
                                    args,
                                };
                                Some(self.subst.unify(&term_arg, &constructed))
                            }
                            _ => Some(false),
                        }
                    }
                }
            }
            BuiltinResult::Arg(n_arg, term_arg, result_arg) => {
                let wn = self.subst.walk(&n_arg);
                let wterm = self.subst.walk(&term_arg);
                if let Term::Integer(n) = wn {
                    if let Term::Compound { args, .. } = &wterm {
                        if n >= 1 && (n as usize) <= args.len() {
                            return Some(self.subst.unify(&result_arg, &args[(n - 1) as usize]));
                        }
                    }
                    if let Term::List { head, tail } = &wterm {
                        return match n {
                            1 => Some(self.subst.unify(&result_arg, head)),
                            2 => Some(self.subst.unify(&result_arg, tail)),
                            _ => Some(false),
                        };
                    }
                }
                Some(false)
            }
            BuiltinResult::Univ(term_arg, list_arg) => {
                let walked = self.subst.walk(&term_arg);
                match &walked {
                    Term::Atom(id) => {
                        let list = build_list(vec![Term::Atom(*id)], &self.interner);
                        Some(self.subst.unify(&list_arg, &list))
                    }
                    Term::Integer(_) | Term::Float(_) => {
                        let list = build_list(vec![walked.clone()], &self.interner);
                        Some(self.subst.unify(&list_arg, &list))
                    }
                    Term::Compound { functor, args } => {
                        let mut elems = vec![Term::Atom(*functor)];
                        elems.extend(args.clone());
                        let list = build_list(elems, &self.interner);
                        Some(self.subst.unify(&list_arg, &list))
                    }
                    Term::List { head, tail } => {
                        let dot_id = self.interner.intern(".");
                        let elems = vec![Term::Atom(dot_id), *head.clone(), *tail.clone()];
                        let list = build_list(elems, &self.interner);
                        Some(self.subst.unify(&list_arg, &list))
                    }
                    Term::Var(_) => {
                        let wlist = self.subst.apply(&list_arg);
                        if let Some(elems) = collect_list(&wlist, &self.interner) {
                            if !elems.is_empty() {
                                if let Term::Atom(fid) = &elems[0] {
                                    if elems.len() == 1 {
                                        return Some(
                                            self.subst.unify(&term_arg, &Term::Atom(*fid)),
                                        );
                                    }
                                    let constructed = Term::Compound {
                                        functor: *fid,
                                        args: elems[1..].to_vec(),
                                    };
                                    return Some(self.subst.unify(&term_arg, &constructed));
                                } else if elems.len() == 1 {
                                    return Some(self.subst.unify(&term_arg, &elems[0]));
                                }
                            }
                        }
                        Some(false)
                    }
                }
            }
            BuiltinResult::CopyTerm(original, copy) => {
                let walked = self.subst.walk(&original);
                let copied = self.copy_term_fresh(&walked);
                Some(self.subst.unify(&copy, &copied))
            }
            BuiltinResult::Succ(x_arg, s_arg) => {
                let wx = self.subst.walk(&x_arg);
                let ws = self.subst.walk(&s_arg);
                match (&wx, &ws) {
                    (Term::Integer(x), _) if *x >= 0 => match x.checked_add(1) {
                        Some(result) => Some(self.subst.unify(&s_arg, &Term::Integer(result))),
                        None => Some(false),
                    },
                    (_, Term::Integer(s)) if *s > 0 => {
                        Some(self.subst.unify(&x_arg, &Term::Integer(s - 1)))
                    }
                    _ => Some(false),
                }
            }
            BuiltinResult::Plus(x_arg, y_arg, z_arg) => {
                let wx = self.subst.walk(&x_arg);
                let wy = self.subst.walk(&y_arg);
                let wz = self.subst.walk(&z_arg);
                match (&wx, &wy, &wz) {
                    (Term::Integer(x), Term::Integer(y), _) => match x.checked_add(*y) {
                        Some(result) => Some(self.subst.unify(&z_arg, &Term::Integer(result))),
                        None => Some(false),
                    },
                    (Term::Integer(x), _, Term::Integer(z)) => match z.checked_sub(*x) {
                        Some(result) => Some(self.subst.unify(&y_arg, &Term::Integer(result))),
                        None => Some(false),
                    },
                    (_, Term::Integer(y), Term::Integer(z)) => match z.checked_sub(*y) {
                        Some(result) => Some(self.subst.unify(&x_arg, &Term::Integer(result))),
                        None => Some(false),
                    },
                    _ => Some(false),
                }
            }
            BuiltinResult::MSort(list_arg, sorted_arg) => {
                let wlist = self.subst.apply(&list_arg);
                if let Some(mut elems) = collect_list(&wlist, &self.interner) {
                    elems.sort_by(|a, b| term_compare(a, b, &self.interner));
                    let sorted = build_list(elems, &self.interner);
                    return Some(self.subst.unify(&sorted_arg, &sorted));
                }
                Some(false)
            }
            BuiltinResult::Sort(list_arg, sorted_arg) => {
                let wlist = self.subst.apply(&list_arg);
                if let Some(mut elems) = collect_list(&wlist, &self.interner) {
                    elems.sort_by(|a, b| term_compare(a, b, &self.interner));
                    elems.dedup_by(|a, b| {
                        term_compare(a, b, &self.interner) == std::cmp::Ordering::Equal
                    });
                    let sorted = build_list(elems, &self.interner);
                    return Some(self.subst.unify(&sorted_arg, &sorted));
                }
                Some(false)
            }
            BuiltinResult::NumberChars(num_arg, chars_arg) => {
                let wnum = self.subst.walk(&num_arg);
                match &wnum {
                    Term::Integer(n) => {
                        let s = n.to_string();
                        let nil_id = self.interner.lookup("[]").expect("[] must be interned");
                        let mut list = Term::Atom(nil_id);
                        for ch in s.chars().rev() {
                            let ch_id = self.interner.intern(&ch.to_string());
                            list = Term::List {
                                head: Box::new(Term::Atom(ch_id)),
                                tail: Box::new(list),
                            };
                        }
                        Some(self.subst.unify(&chars_arg, &list))
                    }
                    Term::Float(f) => {
                        let s = format_float(*f);
                        let nil_id = self.interner.lookup("[]").expect("[] must be interned");
                        let mut list = Term::Atom(nil_id);
                        for ch in s.chars().rev() {
                            let ch_id = self.interner.intern(&ch.to_string());
                            list = Term::List {
                                head: Box::new(Term::Atom(ch_id)),
                                tail: Box::new(list),
                            };
                        }
                        Some(self.subst.unify(&chars_arg, &list))
                    }
                    Term::Var(_) => {
                        // Reverse: char list -> number
                        let wchars = self.subst.apply(&chars_arg);
                        if let Some(elems) = collect_list(&wchars, &self.interner) {
                            let s: Option<String> = elems
                                .iter()
                                .map(|e| match e {
                                    Term::Atom(id) => {
                                        let ch = self.interner.resolve(*id);
                                        if ch.chars().count() == 1 {
                                            Some(ch.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                })
                                .collect();
                            if let Some(s) = s {
                                if let Ok(n) = s.parse::<i64>() {
                                    return Some(self.subst.unify(&num_arg, &Term::Integer(n)));
                                } else if let Ok(f) = s.parse::<f64>() {
                                    return Some(self.subst.unify(&num_arg, &Term::Float(f)));
                                }
                            }
                        }
                        Some(false)
                    }
                    _ => Some(false),
                }
            }
            BuiltinResult::NumberCodes(num_arg, codes_arg) => {
                let wnum = self.subst.walk(&num_arg);
                match &wnum {
                    Term::Integer(n) => {
                        let s = n.to_string();
                        let nil_id = self.interner.lookup("[]").expect("[] must be interned");
                        let mut list = Term::Atom(nil_id);
                        for ch in s.chars().rev() {
                            list = Term::List {
                                head: Box::new(Term::Integer(ch as i64)),
                                tail: Box::new(list),
                            };
                        }
                        Some(self.subst.unify(&codes_arg, &list))
                    }
                    Term::Float(f) => {
                        let s = format_float(*f);
                        let nil_id = self.interner.lookup("[]").expect("[] must be interned");
                        let mut list = Term::Atom(nil_id);
                        for ch in s.chars().rev() {
                            list = Term::List {
                                head: Box::new(Term::Integer(ch as i64)),
                                tail: Box::new(list),
                            };
                        }
                        Some(self.subst.unify(&codes_arg, &list))
                    }
                    Term::Var(_) => {
                        // Reverse: code list -> number
                        let wcodes = self.subst.apply(&codes_arg);
                        if let Some(elems) = collect_list(&wcodes, &self.interner) {
                            let s: Option<String> = elems
                                .iter()
                                .map(|e| {
                                    if let Term::Integer(code) = e {
                                        if *code >= 0 && *code <= 0x10FFFF {
                                            char::from_u32(*code as u32).map(|c| c.to_string())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if let Some(s) = s {
                                if let Ok(n) = s.parse::<i64>() {
                                    return Some(self.subst.unify(&num_arg, &Term::Integer(n)));
                                } else if let Ok(f) = s.parse::<f64>() {
                                    return Some(self.subst.unify(&num_arg, &Term::Float(f)));
                                }
                            }
                        }
                        Some(false)
                    }
                    _ => Some(false),
                }
            }
            _ => None, // Unhandled variants
        }
    }

    /// Copy a term with all variables replaced by fresh ones (for copy_term/2).
    fn copy_term_fresh(&mut self, term: &Term) -> Term {
        let mut var_map: FnvHashMap<VarId, VarId> = FnvHashMap::default();
        self.copy_term_impl(term, &mut var_map)
    }

    fn copy_term_impl(&mut self, term: &Term, var_map: &mut FnvHashMap<VarId, VarId>) -> Term {
        let walked = self.subst.walk(term);
        match &walked {
            Term::Var(id) => {
                let new_id = *var_map.entry(*id).or_insert_with(|| {
                    let fresh = self.var_counter;
                    self.var_counter += 1;
                    fresh
                });
                Term::Var(new_id)
            }
            Term::Atom(_) | Term::Integer(_) | Term::Float(_) => walked.clone(),
            Term::Compound { functor, args } => Term::Compound {
                functor: *functor,
                args: args
                    .iter()
                    .map(|a| self.copy_term_impl(a, var_map))
                    .collect(),
            },
            Term::List { .. } => {
                // Iterative list spine traversal to avoid stack overflow on long lists.
                // We use an owned current value so variable-threaded spines are walked.
                let mut heads = Vec::new();
                let mut current_owned = walked;
                loop {
                    match current_owned {
                        Term::List { head, tail } => {
                            heads.push(self.copy_term_impl(&head, var_map));
                            let walked_tail = self.subst.walk(&tail);
                            match walked_tail {
                                Term::List { .. } => {
                                    // Continue iterating with the walked tail
                                    current_owned = walked_tail;
                                }
                                _ => {
                                    // Terminal element: copy it and build list
                                    let final_tail = self.copy_term_impl(&tail, var_map);
                                    let mut result = final_tail;
                                    for h in heads.into_iter().rev() {
                                        result = Term::List {
                                            head: Box::new(h),
                                            tail: Box::new(result),
                                        };
                                    }
                                    return result;
                                }
                            }
                        }
                        _ => {
                            // Reached a non-list (var, atom, etc.)
                            let final_tail = self.copy_term_impl(&current_owned, var_map);
                            let mut result = final_tail;
                            for h in heads.into_iter().rev() {
                                result = Term::List {
                                    head: Box::new(h),
                                    tail: Box::new(result),
                                };
                            }
                            return result;
                        }
                    }
                }
            }
        }
    }

    /// Extract the current solution based on query variable bindings.
    fn extract_solution(&self) -> Solution {
        let mut bindings = Vec::new();
        let mut vars: Vec<_> = self.query_vars.iter().collect();
        vars.sort_by_key(|(name, _)| name.to_string());
        for (name, &var_id) in vars {
            if name == "_" {
                continue;
            }
            let resolved = self.subst.apply(&Term::Var(var_id));
            bindings.push((name.clone(), resolved));
        }
        Solution { bindings }
    }
}

/// Format a term as a human-readable string.
pub fn term_to_string(term: &Term, interner: &crate::term::StringInterner) -> String {
    match term {
        Term::Atom(id) => interner.resolve(*id).to_string(),
        Term::Var(id) => format!("_{}", id),
        Term::Integer(n) => n.to_string(),
        Term::Float(f) => format!("{}", f),
        Term::Compound { functor, args } => {
            let name = interner.resolve(*functor);
            if args.len() == 2 {
                // Check if it's an infix operator
                match name {
                    "+" | "-" | "*" | "/" | "mod" | "is" | "=" | "\\=" | "<" | ">" | "=<"
                    | ">=" | "=:=" | "=\\=" => {
                        return format!(
                            "{} {} {}",
                            term_to_string(&args[0], interner),
                            name,
                            term_to_string(&args[1], interner)
                        );
                    }
                    _ => {}
                }
            }
            format!(
                "{}({})",
                name,
                args.iter()
                    .map(|a| term_to_string(a, interner))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        Term::List { head, tail } => {
            let mut elements = vec![term_to_string(head, interner)];
            let mut current = tail.as_ref();
            loop {
                match current {
                    Term::List { head, tail } => {
                        elements.push(term_to_string(head, interner));
                        current = tail;
                    }
                    Term::Atom(id) if interner.resolve(*id) == "[]" => {
                        return format!("[{}]", elements.join(", "));
                    }
                    _ => {
                        return format!(
                            "[{}|{}]",
                            elements.join(", "),
                            term_to_string(current, interner)
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CompiledDatabase;
    use crate::parser::Parser;

    fn query(source: &str, query_str: &str) -> Vec<Solution> {
        let mut interner = crate::term::StringInterner::new();
        let clauses = Parser::parse_program(source, &mut interner).unwrap();
        let (goals, vars) = Parser::parse_query_with_vars(query_str, &mut interner).unwrap();
        let db = CompiledDatabase::new(interner, clauses);
        let solver = Solver::new(&db, goals, vars);
        solver.all_solutions().unwrap()
    }

    fn query_first_binding(source: &str, query_str: &str, var_name: &str) -> Option<String> {
        let mut interner = crate::term::StringInterner::new();
        let clauses = Parser::parse_program(source, &mut interner).unwrap();
        let (goals, vars) = Parser::parse_query_with_vars(query_str, &mut interner).unwrap();
        let db = CompiledDatabase::new(interner, clauses);
        let solver = Solver::new(&db, goals, vars);
        let (solutions, solver_interner) = solver.all_solutions_with_interner().unwrap();
        solutions.first().and_then(|sol| {
            sol.bindings
                .iter()
                .find(|(name, _)| name == var_name)
                .map(|(_, term)| term_to_string(term, &solver_interner))
        })
    }

    #[test]
    fn test_simple_fact() {
        let solutions = query("likes(mary, food).", "likes(mary, food)");
        assert_eq!(solutions.len(), 1);
        assert!(solutions[0].bindings.is_empty()); // no variables
    }

    #[test]
    fn test_fact_negative() {
        let solutions = query("likes(mary, food).", "likes(mary, beer)");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_variable_binding() {
        let result = query_first_binding("likes(mary, food).", "likes(mary, X)", "X");
        assert_eq!(result, Some("food".to_string()));
    }

    #[test]
    fn test_simple_rule() {
        let source = "likes(mary, food). happy(X) :- likes(X, food).";
        let solutions = query(source, "happy(mary)");
        assert_eq!(solutions.len(), 1);
    }

    #[test]
    fn test_multiple_solutions() {
        let source = "parent(tom, mary). parent(tom, james). parent(tom, ann).";
        let solutions = query(source, "parent(tom, X)");
        assert_eq!(solutions.len(), 3);
    }

    #[test]
    fn test_recursive_rule() {
        let source = r#"
            parent(tom, mary).
            parent(mary, ann).
            ancestor(X, Y) :- parent(X, Y).
            ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).
        "#;
        let solutions = query(source, "ancestor(tom, ann)");
        assert!(solutions.len() >= 1);
    }

    #[test]
    fn test_grandparent() {
        let source = r#"
            parent(tom, mary).
            parent(mary, ann).
            grandparent(X, Z) :- parent(X, Y), parent(Y, Z).
        "#;
        let result = query_first_binding(source, "grandparent(tom, X)", "X");
        assert_eq!(result, Some("ann".to_string()));
    }

    #[test]
    fn test_arithmetic_is() {
        let source = "add(X, Y, Z) :- Z is X + Y.";
        let result = query_first_binding(source, "add(3, 4, X)", "X");
        assert_eq!(result, Some("7".to_string()));
    }

    #[test]
    fn test_arithmetic_multiply() {
        let source = "double(X, Y) :- Y is X * 2.";
        let result = query_first_binding(source, "double(5, X)", "X");
        assert_eq!(result, Some("10".to_string()));
    }

    #[test]
    fn test_comparison_positive() {
        let source = "big(X) :- X > 100.";
        let solutions = query(source, "big(200)");
        assert_eq!(solutions.len(), 1);
    }

    #[test]
    fn test_comparison_negative() {
        let source = "big(X) :- X > 100.";
        let solutions = query(source, "big(50)");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_operator_precedence() {
        let source = "result(X) :- X is 2 + 3 * 4.";
        let result = query_first_binding(source, "result(X)", "X");
        assert_eq!(result, Some("14".to_string()));
    }

    #[test]
    fn test_mod_operator() {
        let source = "remainder(X, Y, Z) :- Z is X mod Y.";
        let result = query_first_binding(source, "remainder(10, 3, X)", "X");
        assert_eq!(result, Some("1".to_string()));
    }

    #[test]
    fn test_nested_compound() {
        let source = "outer(inner(deep(hello))).";
        let solutions = query(source, "outer(inner(deep(hello)))");
        assert_eq!(solutions.len(), 1);
    }

    #[test]
    fn test_nested_compound_with_var() {
        let source = "outer(inner(deep(hello))).";
        let result = query_first_binding(source, "outer(inner(deep(X)))", "X");
        assert_eq!(result, Some("hello".to_string()));
    }

    #[test]
    fn test_negation_as_failure() {
        let source = r#"
            likes(mary, food).
            likes(mary, wine).
            dislikes(X, Y) :- \+ likes(X, Y).
        "#;
        let solutions = query(source, "dislikes(mary, beer)");
        assert_eq!(solutions.len(), 1);

        let solutions = query(source, "dislikes(mary, food)");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_cut() {
        let source = r#"
            max(X, Y, X) :- X >= Y, !.
            max(X, Y, Y).
        "#;
        // max(3, 5, Z) should give Z = 5 (first clause fails, second succeeds)
        let result = query_first_binding(source, "max(3, 5, Z)", "Z");
        assert_eq!(result, Some("5".to_string()));

        // max(5, 3, Z) should give Z = 5 (first clause succeeds + cut)
        let result = query_first_binding(source, "max(5, 3, Z)", "Z");
        assert_eq!(result, Some("5".to_string()));
    }

    #[test]
    fn test_index_multi_predicate() {
        let source = "color(red). color(blue). color(green). shape(circle). shape(square).";
        let solutions = query(source, "shape(circle)");
        assert_eq!(solutions.len(), 1);

        let solutions = query(source, "color(circle)");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_index_first_arg() {
        let source = r#"
            component(engine, piston).
            component(engine, crankshaft).
            component(engine, valve).
            component(brake, pad).
            component(brake, rotor).
            component(wheel, tire).
            component(wheel, rim).
        "#;
        let result = query_first_binding(source, "component(brake, X)", "X");
        assert_eq!(result, Some("pad".to_string()));

        let solutions = query(source, "component(engine, X)");
        assert_eq!(solutions.len(), 3);

        let solutions = query(source, "component(transmission, X)");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_index_variable_fallback() {
        let source = r#"
            component(engine, piston).
            component(brake, pad).
            component(wheel, tire).
        "#;
        let solutions = query(source, "component(X, tire)");
        assert!(solutions.len() >= 1);
        let result = query_first_binding(source, "component(X, tire)", "X");
        assert_eq!(result, Some("wheel".to_string()));
    }

    #[test]
    fn test_mixed_ground_var_clauses() {
        let source = r#"
            lookup(a, 1).
            lookup(b, 2).
            lookup(c, 3).
            lookup(X, 0) :- X = default.
        "#;
        let result = query_first_binding(source, "lookup(b, X)", "X");
        assert_eq!(result, Some("2".to_string()));

        let result = query_first_binding(source, "lookup(default, X)", "X");
        assert_eq!(result, Some("0".to_string()));
    }

    #[test]
    fn test_disjunction() {
        let source = "color(red). color(blue).";
        let solutions = query(source, "( color(red) ; color(green) )");
        assert_eq!(solutions.len(), 1);
    }

    #[test]
    fn test_disjunction_both_branches() {
        let source = "color(red). color(blue).";
        let solutions = query(source, "( color(red) ; color(blue) )");
        assert_eq!(solutions.len(), 2);
    }

    #[test]
    fn test_if_then_else_true() {
        // If 1 < 2, then X = yes, else X = no
        let source = "";
        let result = query_first_binding(source, "(1 < 2 -> X = yes ; X = no)", "X");
        assert_eq!(result, Some("yes".to_string()));
    }

    #[test]
    fn test_if_then_else_false() {
        let source = "";
        let result = query_first_binding(source, "(2 < 1 -> X = yes ; X = no)", "X");
        assert_eq!(result, Some("no".to_string()));
    }

    #[test]
    fn test_findall_basic() {
        let source = "color(red). color(green). color(blue).";
        let result = query_first_binding(source, "findall(X, color(X), L)", "L");
        assert_eq!(result, Some("[red, green, blue]".to_string()));
    }

    #[test]
    fn test_findall_empty() {
        let source = "color(red).";
        let result = query_first_binding(source, "findall(X, shape(X), L)", "L");
        assert_eq!(result, Some("[]".to_string()));
    }

    #[test]
    fn test_findall_with_rule() {
        let source = r#"
            parent(tom, mary). parent(tom, james). parent(tom, ann).
        "#;
        let result = query_first_binding(source, "findall(C, parent(tom, C), Kids)", "Kids");
        assert_eq!(result, Some("[mary, james, ann]".to_string()));
    }

    #[test]
    fn test_if_then_no_else() {
        let source = "";
        let solutions = query(source, "(1 < 2 -> true)");
        assert_eq!(solutions.len(), 1);

        let solutions = query(source, "(2 < 1 -> true)");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_solution_limit() {
        let source = "n(1). n(2). n(3). n(4). n(5).";
        let mut interner = crate::term::StringInterner::new();
        let clauses = Parser::parse_program(source, &mut interner).unwrap();
        let (goals, vars) = Parser::parse_query_with_vars("n(X)", &mut interner).unwrap();
        let db = CompiledDatabase::new(interner, clauses);
        let solver = Solver::new(&db, goals, vars).with_limit(3);
        let solutions = solver.all_solutions().unwrap();
        assert_eq!(solutions.len(), 3);
    }

    #[test]
    fn test_depth_limit() {
        // Create an infinite loop: loop :- loop.
        let source = "loop :- loop.";
        let mut interner = crate::term::StringInterner::new();
        let clauses = Parser::parse_program(source, &mut interner).unwrap();
        let (goals, vars) = Parser::parse_query_with_vars("loop", &mut interner).unwrap();
        let db = CompiledDatabase::new(interner, clauses);
        let mut solver = Solver::new(&db, goals, vars).with_max_depth(100);
        let result = solver.next();
        assert!(matches!(result, SolveResult::Error(ref e) if e.contains("step limit")));
    }

    // Phase 3 tests: once/1, call/1, atom predicates, arithmetic functions

    #[test]
    fn test_once_basic() {
        let source = "color(red). color(green). color(blue).";
        // once/1 should return only the first solution
        let solutions = query(source, "once(color(X))");
        assert_eq!(solutions.len(), 1);
    }

    #[test]
    fn test_once_prevents_backtracking() {
        let source = "n(1). n(2). n(3).";
        let result = query_first_binding(source, "once(n(X))", "X");
        assert_eq!(result, Some("1".to_string()));
    }

    #[test]
    fn test_once_fails_if_goal_fails() {
        let source = "color(red).";
        let solutions = query(source, "once(shape(X))");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_call_basic() {
        let source = "color(red). color(blue).";
        let solutions = query(source, "call(color(red))");
        assert_eq!(solutions.len(), 1);
    }

    #[test]
    fn test_call_with_variable() {
        let source = "color(red). color(blue). color(green).";
        let solutions = query(source, "call(color(X))");
        assert_eq!(solutions.len(), 3);
    }

    #[test]
    fn test_call_fails() {
        let source = "color(red).";
        let solutions = query(source, "call(shape(X))");
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn test_atom_length() {
        let source = "";
        let result = query_first_binding(source, "atom_length(hello, N)", "N");
        assert_eq!(result, Some("5".to_string()));
    }

    #[test]
    fn test_atom_length_empty() {
        let source = "";
        // Parse an empty atom using quotes
        let result = query_first_binding(source, "atom_length('', N)", "N");
        assert_eq!(result, Some("0".to_string()));
    }

    #[test]
    fn test_atom_concat() {
        let source = "";
        let result = query_first_binding(source, "atom_concat(hello, world, X)", "X");
        assert_eq!(result, Some("helloworld".to_string()));
    }

    #[test]
    fn test_atom_chars() {
        let source = "";
        let result = query_first_binding(source, "atom_chars(abc, X)", "X");
        assert_eq!(result, Some("[a, b, c]".to_string()));
    }

    #[test]
    fn test_atom_chars_single() {
        let source = "";
        let result = query_first_binding(source, "atom_chars(x, X)", "X");
        assert_eq!(result, Some("[x]".to_string()));
    }

    #[test]
    fn test_arith_abs_in_rule() {
        let source = "dist(X, Y, D) :- D is abs(X - Y).";
        let result = query_first_binding(source, "dist(3, 7, D)", "D");
        assert_eq!(result, Some("4".to_string()));
    }

    #[test]
    fn test_arith_max_in_rule() {
        let source = "bigger(X, Y, Z) :- Z is max(X, Y).";
        let result = query_first_binding(source, "bigger(3, 7, Z)", "Z");
        assert_eq!(result, Some("7".to_string()));
    }

    #[test]
    fn test_arith_min_in_rule() {
        let source = "smaller(X, Y, Z) :- Z is min(X, Y).";
        let result = query_first_binding(source, "smaller(3, 7, Z)", "Z");
        assert_eq!(result, Some("3".to_string()));
    }

    #[test]
    fn test_arith_sign_in_rule() {
        let source = "direction(X, S) :- S is sign(X).";
        let result = query_first_binding(source, "direction(-42, S)", "S");
        assert_eq!(result, Some("-1".to_string()));
    }
}
