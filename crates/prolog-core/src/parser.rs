use crate::term::{Clause, StringInterner, Term, VarId};
use crate::tokenizer::{Token, TokenKind, Tokenizer};
use fnv::FnvHashMap;

/// Parser for Edinburgh Prolog syntax.
/// Parses tokens into Terms and Clauses, with variable scoping per clause.
pub struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    interner: &'a mut StringInterner,
    var_map: FnvHashMap<String, VarId>,
    next_var: VarId,
}

impl<'a> Parser<'a> {
    /// Parse a complete program (multiple clauses) from source text.
    pub fn parse_program(input: &str, interner: &mut StringInterner) -> Result<Vec<Clause>, String> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser {
            tokens,
            pos: 0,
            interner,
            var_map: FnvHashMap::default(),
            next_var: 0,
        };
        let mut clauses = Vec::new();
        while !parser.at_eof() {
            parser.reset_vars();
            let clause = parser.parse_clause()?;
            clauses.push(clause);
        }
        Ok(clauses)
    }

    /// Parse a single query (goal list) from source text, e.g. "parent(tom, X)".
    /// Does NOT require a trailing dot.
    pub fn parse_query(input: &str, interner: &mut StringInterner) -> Result<Vec<Term>, String> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser {
            tokens,
            pos: 0,
            interner,
            var_map: FnvHashMap::default(),
            next_var: 0,
        };
        // Skip optional ?- prefix
        if parser.current_kind() == Some(&TokenKind::QueryOp) {
            parser.advance();
        }
        let goals = parser.parse_goal_list()?;
        // Allow optional trailing dot
        if parser.current_kind() == Some(&TokenKind::Dot) {
            parser.advance();
        }
        Ok(goals)
    }

    fn reset_vars(&mut self) {
        self.var_map.clear();
        self.next_var = 0;
    }

    fn current(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn current_kind(&self) -> Option<&TokenKind> {
        self.current().map(|t| &t.kind)
    }

    fn at_eof(&self) -> bool {
        matches!(self.current_kind(), None | Some(TokenKind::Eof))
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        self.pos += 1;
        tok
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<(), String> {
        match self.current() {
            Some(tok) if &tok.kind == kind => {
                self.advance();
                Ok(())
            }
            Some(tok) => Err(format!(
                "Expected {:?}, got {:?} at line {} col {}",
                kind, tok.kind, tok.line, tok.col
            )),
            None => Err(format!("Expected {:?}, got end of input", kind)),
        }
    }

    fn parse_clause(&mut self) -> Result<Clause, String> {
        let head = self.parse_term()?;
        match self.current_kind() {
            Some(TokenKind::Dot) => {
                self.advance();
                Ok(Clause { head, body: vec![] })
            }
            Some(TokenKind::Neck) => {
                self.advance();
                let body = self.parse_goal_list()?;
                self.expect(&TokenKind::Dot)?;
                Ok(Clause { head, body })
            }
            Some(tok) => {
                let tok = tok.clone();
                Err(format!("Expected '.' or ':-', got {:?} at line {} col {}",
                    tok, self.current().unwrap().line, self.current().unwrap().col))
            }
            None => Err("Unexpected end of input in clause".to_string()),
        }
    }

    fn parse_goal_list(&mut self) -> Result<Vec<Term>, String> {
        let mut goals = vec![self.parse_goal_disjunction()?];
        while self.current_kind() == Some(&TokenKind::Comma) {
            self.advance();
            goals.push(self.parse_goal_disjunction()?);
        }
        Ok(goals)
    }

    /// Parse disjunction (;) at goal level — lower precedence than comma.
    fn parse_goal_disjunction(&mut self) -> Result<Term, String> {
        let left = self.parse_term()?;
        if self.current_kind() == Some(&TokenKind::Semicolon) {
            self.advance();
            let right = self.parse_goal_disjunction()?;
            let functor = self.interner.intern(";");
            Ok(Term::Compound {
                functor,
                args: vec![left, right],
            })
        } else {
            Ok(left)
        }
    }

    /// Parse a term at the top level (precedence 700 — non-associative comparison/is level).
    fn parse_term(&mut self) -> Result<Term, String> {
        self.parse_expr_700()
    }

    /// Precedence 700: non-associative operators (is, =, \=, <, >, =<, >=, =:=, =\=)
    fn parse_expr_700(&mut self) -> Result<Term, String> {
        let left = self.parse_expr_500()?;
        if let Some(op) = self.match_op_700() {
            let right = self.parse_expr_500()?;
            Ok(self.build_binop(&op, left, right))
        } else {
            Ok(left)
        }
    }

    fn match_op_700(&mut self) -> Option<String> {
        let op = match self.current_kind()? {
            TokenKind::Is => "is",
            TokenKind::Equals => "=",
            TokenKind::NotEquals => "\\=",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::Lte => "=<",
            TokenKind::Gte => ">=",
            TokenKind::ArithEq => "=:=",
            TokenKind::ArithNeq => "=\\=",
            _ => return None,
        };
        self.advance();
        Some(op.to_string())
    }

    /// Precedence 500: left-associative (+, -)
    fn parse_expr_500(&mut self) -> Result<Term, String> {
        let mut left = self.parse_expr_400()?;
        loop {
            let op = match self.current_kind() {
                Some(TokenKind::Plus) => "+",
                Some(TokenKind::Minus) => "-",
                _ => break,
            };
            let op = op.to_string();
            self.advance();
            let right = self.parse_expr_400()?;
            left = self.build_binop(&op, left, right);
        }
        Ok(left)
    }

    /// Precedence 400: left-associative (*, /, mod)
    fn parse_expr_400(&mut self) -> Result<Term, String> {
        let mut left = self.parse_primary()?;
        loop {
            let op = match self.current_kind() {
                Some(TokenKind::Star) => "*",
                Some(TokenKind::Slash) => "/",
                Some(TokenKind::Mod) => "mod",
                _ => break,
            };
            let op = op.to_string();
            self.advance();
            let right = self.parse_primary()?;
            left = self.build_binop(&op, left, right);
        }
        Ok(left)
    }

    fn build_binop(&mut self, op: &str, left: Term, right: Term) -> Term {
        let functor = self.interner.intern(op);
        Term::Compound {
            functor,
            args: vec![left, right],
        }
    }

    fn parse_primary(&mut self) -> Result<Term, String> {
        match self.current_kind().cloned() {
            Some(TokenKind::Integer(n)) => {
                self.advance();
                Ok(Term::Integer(n))
            }
            Some(TokenKind::Float(f)) => {
                self.advance();
                Ok(Term::Float(f))
            }
            Some(TokenKind::Variable(ref name)) => {
                let name = name.clone();
                self.advance();
                if name == "_" {
                    // Anonymous variable — always fresh
                    let id = self.next_var;
                    self.next_var += 1;
                    Ok(Term::Var(id))
                } else if let Some(&id) = self.var_map.get(&name) {
                    Ok(Term::Var(id))
                } else {
                    let id = self.next_var;
                    self.next_var += 1;
                    self.var_map.insert(name, id);
                    Ok(Term::Var(id))
                }
            }
            Some(TokenKind::Atom(ref name)) => {
                let name = name.clone();
                self.advance();
                // Check if followed by '(' — compound term
                if self.current_kind() == Some(&TokenKind::LParen) {
                    self.advance(); // skip (
                    let args = self.parse_arg_list()?;
                    self.expect(&TokenKind::RParen)?;
                    let functor = self.interner.intern(&name);
                    Ok(Term::Compound { functor, args })
                } else {
                    let id = self.interner.intern(&name);
                    Ok(Term::Atom(id))
                }
            }
            Some(TokenKind::LParen) => {
                self.advance();
                let term = self.parse_paren_body()?;
                self.expect(&TokenKind::RParen)?;
                Ok(term)
            }
            Some(TokenKind::Minus) => {
                self.advance();
                let operand = self.parse_primary()?;
                // Optimize: if operand is a literal number, negate it directly
                match operand {
                    Term::Integer(n) => Ok(Term::Integer(-n)),
                    Term::Float(f) => Ok(Term::Float(-f)),
                    _ => {
                        let functor = self.interner.intern("-");
                        Ok(Term::Compound {
                            functor,
                            args: vec![operand],
                        })
                    }
                }
            }
            Some(TokenKind::LBracket) => {
                self.advance(); // skip [
                self.parse_list_body()
            }
            Some(TokenKind::Cut) => {
                self.advance();
                let id = self.interner.intern("!");
                Ok(Term::Atom(id))
            }
            Some(TokenKind::Not) => {
                // \+ Goal
                self.advance();
                let goal = self.parse_primary()?;
                let functor = self.interner.intern("\\+");
                Ok(Term::Compound {
                    functor,
                    args: vec![goal],
                })
            }
            Some(ref tok) => {
                let msg = format!("Unexpected token {:?} at line {} col {}",
                    tok, self.current().unwrap().line, self.current().unwrap().col);
                Err(msg)
            }
            None => Err("Unexpected end of input".to_string()),
        }
    }

    /// Parse the body of a parenthesized expression, handling ; and ->.
    /// Supports: (A ; B), (Cond -> Then), (Cond -> Then ; Else)
    fn parse_paren_body(&mut self) -> Result<Term, String> {
        let first = self.parse_paren_comma_list()?;

        if self.current_kind() == Some(&TokenKind::Arrow) {
            // (Cond -> Then) or (Cond -> Then ; Else)
            self.advance();
            let then = self.parse_paren_comma_list()?;
            let arrow_functor = self.interner.intern("->");
            let if_then = Term::Compound {
                functor: arrow_functor,
                args: vec![first, then],
            };
            if self.current_kind() == Some(&TokenKind::Semicolon) {
                self.advance();
                let else_branch = self.parse_paren_body()?;
                let semi_functor = self.interner.intern(";");
                Ok(Term::Compound {
                    functor: semi_functor,
                    args: vec![if_then, else_branch],
                })
            } else {
                Ok(if_then)
            }
        } else if self.current_kind() == Some(&TokenKind::Semicolon) {
            // (A ; B)
            self.advance();
            let right = self.parse_paren_body()?;
            let functor = self.interner.intern(";");
            Ok(Term::Compound {
                functor,
                args: vec![first, right],
            })
        } else {
            Ok(first)
        }
    }

    /// Parse a comma-separated goal conjunction within parens, building ','(A,B) terms.
    fn parse_paren_comma_list(&mut self) -> Result<Term, String> {
        let first = self.parse_term()?;
        if self.current_kind() == Some(&TokenKind::Comma) {
            // Check that the next comma isn't just the end of an arg list —
            // but inside parens for ; / ->, comma means conjunction
            self.advance();
            let rest = self.parse_paren_comma_list()?;
            let functor = self.interner.intern(",");
            Ok(Term::Compound {
                functor,
                args: vec![first, rest],
            })
        } else {
            Ok(first)
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Term>, String> {
        let mut args = vec![self.parse_term()?];
        while self.current_kind() == Some(&TokenKind::Comma) {
            self.advance();
            args.push(self.parse_term()?);
        }
        Ok(args)
    }

    fn parse_list_body(&mut self) -> Result<Term, String> {
        // We're right after '['. Parse list elements.
        if self.current_kind() == Some(&TokenKind::RBracket) {
            self.advance();
            let nil = self.interner.intern("[]");
            return Ok(Term::Atom(nil));
        }

        let first = self.parse_term()?;
        self.parse_list_tail(first)
    }

    fn parse_list_tail(&mut self, head: Term) -> Result<Term, String> {
        match self.current_kind() {
            Some(TokenKind::Comma) => {
                self.advance();
                let next_head = self.parse_term()?;
                let tail = self.parse_list_tail(next_head)?;
                Ok(Term::List {
                    head: Box::new(head),
                    tail: Box::new(tail),
                })
            }
            Some(TokenKind::Pipe) => {
                self.advance();
                let tail = self.parse_term()?;
                self.expect(&TokenKind::RBracket)?;
                Ok(Term::List {
                    head: Box::new(head),
                    tail: Box::new(tail),
                })
            }
            Some(TokenKind::RBracket) => {
                self.advance();
                let nil = self.interner.intern("[]");
                Ok(Term::List {
                    head: Box::new(head),
                    tail: Box::new(Term::Atom(nil)),
                })
            }
            _ => Err("Expected ',', '|', or ']' in list".to_string()),
        }
    }

    /// Get the variable name map (for extracting query variable names in results).
    pub fn var_names(&self) -> &FnvHashMap<String, VarId> {
        &self.var_map
    }

    /// Parse a query and also return the variable name mapping.
    pub fn parse_query_with_vars(
        input: &str,
        interner: &mut StringInterner,
    ) -> Result<(Vec<Term>, FnvHashMap<String, VarId>), String> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser {
            tokens,
            pos: 0,
            interner,
            var_map: FnvHashMap::default(),
            next_var: 0,
        };
        if parser.current_kind() == Some(&TokenKind::QueryOp) {
            parser.advance();
        }
        let goals = parser.parse_goal_list()?;
        if parser.current_kind() == Some(&TokenKind::Dot) {
            parser.advance();
        }
        let vars = parser.var_map;
        Ok((goals, vars))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_term(input: &str) -> (Term, StringInterner) {
        let mut interner = StringInterner::new();
        let goals = Parser::parse_query(input, &mut interner).unwrap();
        assert_eq!(goals.len(), 1);
        (goals.into_iter().next().unwrap(), interner)
    }

    fn parse_clauses(input: &str) -> (Vec<Clause>, StringInterner) {
        let mut interner = StringInterner::new();
        let clauses = Parser::parse_program(input, &mut interner).unwrap();
        (clauses, interner)
    }

    #[test]
    fn test_parse_atom() {
        let (term, interner) = parse_term("hello");
        match term {
            Term::Atom(id) => assert_eq!(interner.resolve(id), "hello"),
            _ => panic!("Expected atom"),
        }
    }

    #[test]
    fn test_parse_integer() {
        let (term, _) = parse_term("42");
        assert_eq!(term, Term::Integer(42));
    }

    #[test]
    fn test_parse_float() {
        let (term, _) = parse_term("3.14");
        assert_eq!(term, Term::Float(3.14));
    }

    #[test]
    fn test_parse_variable() {
        let (term, _) = parse_term("X");
        match term {
            Term::Var(_) => {}
            _ => panic!("Expected variable"),
        }
    }

    #[test]
    fn test_parse_compound() {
        let (term, interner) = parse_term("parent(tom, mary)");
        match term {
            Term::Compound { functor, args } => {
                assert_eq!(interner.resolve(functor), "parent");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected compound"),
        }
    }

    #[test]
    fn test_parse_nested_compound() {
        let (term, interner) = parse_term("outer(inner(deep(hello)))");
        match term {
            Term::Compound { functor, ref args } => {
                assert_eq!(interner.resolve(functor), "outer");
                match &args[0] {
                    Term::Compound { functor, ref args } => {
                        assert_eq!(interner.resolve(*functor), "inner");
                        match &args[0] {
                            Term::Compound { functor, ref args } => {
                                assert_eq!(interner.resolve(*functor), "deep");
                                match &args[0] {
                                    Term::Atom(id) => assert_eq!(interner.resolve(*id), "hello"),
                                    _ => panic!("Expected atom"),
                                }
                            }
                            _ => panic!("Expected compound"),
                        }
                    }
                    _ => panic!("Expected compound"),
                }
            }
            _ => panic!("Expected compound"),
        }
    }

    #[test]
    fn test_parse_fact() {
        let (clauses, interner) = parse_clauses("likes(mary, food).");
        assert_eq!(clauses.len(), 1);
        assert!(clauses[0].body.is_empty());
        match &clauses[0].head {
            Term::Compound { functor, args } => {
                assert_eq!(interner.resolve(*functor), "likes");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected compound"),
        }
    }

    #[test]
    fn test_parse_rule() {
        let (clauses, interner) = parse_clauses("happy(X) :- likes(X, food).");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].body.len(), 1);
        match &clauses[0].head {
            Term::Compound { functor, .. } => {
                assert_eq!(interner.resolve(*functor), "happy");
            }
            _ => panic!("Expected compound"),
        }
    }

    #[test]
    fn test_variable_scoping() {
        // Same variable name within a clause should get same id
        let (clauses, _) = parse_clauses("foo(X, Y) :- bar(X, Y).");
        let clause = &clauses[0];
        // Extract var ids from head
        if let Term::Compound { args: head_args, .. } = &clause.head {
            if let (Term::Var(hx), Term::Var(hy)) = (&head_args[0], &head_args[1]) {
                // Same vars in body
                if let Term::Compound { args: body_args, .. } = &clause.body[0] {
                    if let (Term::Var(bx), Term::Var(by)) = (&body_args[0], &body_args[1]) {
                        assert_eq!(hx, bx, "X in head and body should be same var");
                        assert_eq!(hy, by, "Y in head and body should be same var");
                        assert_ne!(hx, hy, "X and Y should be different vars");
                    }
                }
            }
        }
    }

    #[test]
    fn test_operator_precedence() {
        // 2 + 3 * 4 should parse as 2 + (3 * 4)
        let (term, interner) = parse_term("2 + 3 * 4");
        match term {
            Term::Compound { functor, ref args } => {
                assert_eq!(interner.resolve(functor), "+");
                assert_eq!(args[0], Term::Integer(2));
                match &args[1] {
                    Term::Compound { functor, ref args } => {
                        assert_eq!(interner.resolve(*functor), "*");
                        assert_eq!(args[0], Term::Integer(3));
                        assert_eq!(args[1], Term::Integer(4));
                    }
                    _ => panic!("Expected compound for 3*4"),
                }
            }
            _ => panic!("Expected compound for addition"),
        }
    }

    #[test]
    fn test_parenthesized_expr() {
        // (2 + 3) * 4 should parse as (2 + 3) * 4
        let (term, interner) = parse_term("(2 + 3) * 4");
        match term {
            Term::Compound { functor, ref args } => {
                assert_eq!(interner.resolve(functor), "*");
                match &args[0] {
                    Term::Compound { functor, ref args } => {
                        assert_eq!(interner.resolve(*functor), "+");
                        assert_eq!(args[0], Term::Integer(2));
                        assert_eq!(args[1], Term::Integer(3));
                    }
                    _ => panic!("Expected compound for addition"),
                }
                assert_eq!(args[1], Term::Integer(4));
            }
            _ => panic!("Expected compound for multiplication"),
        }
    }

    #[test]
    fn test_is_expression() {
        let (term, interner) = parse_term("X is 2 + 3");
        match term {
            Term::Compound { functor, args } => {
                assert_eq!(interner.resolve(functor), "is");
                assert!(matches!(args[0], Term::Var(_)));
                match &args[1] {
                    Term::Compound { functor, .. } => {
                        assert_eq!(interner.resolve(*functor), "+");
                    }
                    _ => panic!("Expected compound"),
                }
            }
            _ => panic!("Expected compound"),
        }
    }

    #[test]
    fn test_unary_minus() {
        let (term, _) = parse_term("- 5");
        assert_eq!(term, Term::Integer(-5));
    }

    #[test]
    fn test_empty_list() {
        let (term, interner) = parse_term("[]");
        match term {
            Term::Atom(id) => assert_eq!(interner.resolve(id), "[]"),
            _ => panic!("Expected empty list atom"),
        }
    }

    #[test]
    fn test_simple_list() {
        let (term, interner) = parse_term("[1, 2, 3]");
        // Should be List(1, List(2, List(3, Atom([]))))
        match term {
            Term::List { ref head, ref tail } => {
                assert_eq!(**head, Term::Integer(1));
                match tail.as_ref() {
                    Term::List { ref head, ref tail } => {
                        assert_eq!(**head, Term::Integer(2));
                        match tail.as_ref() {
                            Term::List { ref head, ref tail } => {
                                assert_eq!(**head, Term::Integer(3));
                                match tail.as_ref() {
                                    Term::Atom(id) => assert_eq!(interner.resolve(*id), "[]"),
                                    _ => panic!("Expected nil"),
                                }
                            }
                            _ => panic!("Expected list"),
                        }
                    }
                    _ => panic!("Expected list"),
                }
            }
            _ => panic!("Expected list, got {:?}", term),
        }
    }

    #[test]
    fn test_head_tail_list() {
        let (term, _) = parse_term("[H | T]");
        match term {
            Term::List { head, tail } => {
                assert!(matches!(*head, Term::Var(_)));
                assert!(matches!(*tail, Term::Var(_)));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_multiple_clauses() {
        let (clauses, _) = parse_clauses("a. b. c.");
        assert_eq!(clauses.len(), 3);
    }

    #[test]
    fn test_parse_error() {
        let mut interner = StringInterner::new();
        let result = Parser::parse_program("invalid(((.", &mut interner);
        assert!(result.is_err());
    }

    #[test]
    fn test_comparison_operators() {
        let (term, interner) = parse_term("X > 100");
        match term {
            Term::Compound { functor, .. } => {
                assert_eq!(interner.resolve(functor), ">");
            }
            _ => panic!("Expected compound"),
        }
    }

    #[test]
    fn test_cut() {
        let (clauses, interner) = parse_clauses("max(X, Y, X) :- X >= Y, !.");
        assert_eq!(clauses[0].body.len(), 2);
        match &clauses[0].body[1] {
            Term::Atom(id) => assert_eq!(interner.resolve(*id), "!"),
            _ => panic!("Expected cut atom"),
        }
    }

    #[test]
    fn test_negation() {
        let (term, interner) = parse_term("\\+ foo(X)");
        match term {
            Term::Compound { functor, args } => {
                assert_eq!(interner.resolve(functor), "\\+");
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected compound"),
        }
    }
}
