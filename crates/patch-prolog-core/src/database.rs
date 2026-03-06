use crate::index::{build_index, lookup_clauses, PredicateIndex};
use crate::term::{Clause, StringInterner, Term};
use serde::{Deserialize, Serialize};

/// A compiled, indexed Prolog knowledge base.
/// Built at compile time, serialized with bincode, and embedded in the binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledDatabase {
    pub interner: StringInterner,
    pub clauses: Vec<Clause>,
    pub predicate_index: PredicateIndex,
}

impl CompiledDatabase {
    /// Build a compiled database from clauses and interner.
    pub fn new(mut interner: StringInterner, clauses: Vec<Clause>) -> Self {
        // Ensure required atoms are always interned
        interner.intern("[]");
        interner.intern("!");
        let predicate_index = build_index(&clauses);
        CompiledDatabase {
            interner,
            clauses,
            predicate_index,
        }
    }

    /// Look up candidate clause indices for a goal.
    pub fn lookup(&self, goal: &Term) -> Vec<usize> {
        lookup_clauses(&self.predicate_index, goal, &self.clauses)
    }

    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        bincode::serialize(self).map_err(|e| format!("Serialization error: {}", e))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        bincode::deserialize(data).map_err(|e| format!("Deserialization error: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn build_db(source: &str) -> CompiledDatabase {
        let mut interner = StringInterner::new();
        let clauses = Parser::parse_program(source, &mut interner).unwrap();
        CompiledDatabase::new(interner, clauses)
    }

    #[test]
    fn test_roundtrip_serialization() {
        let db = build_db("parent(tom, mary). parent(tom, james).");
        let bytes = db.to_bytes().unwrap();
        let restored = CompiledDatabase::from_bytes(&bytes).unwrap();
        assert_eq!(restored.clauses.len(), 2);
        assert_eq!(restored.interner.resolve(0), db.interner.resolve(0));
    }

    #[test]
    fn test_lookup_indexed() {
        let db = build_db("color(red). color(blue). color(green). shape(circle). shape(square).");
        // color/1 should have 3 clauses
        let color_id = db.interner.lookup("color").unwrap();
        let goal = Term::Compound {
            functor: color_id,
            args: vec![Term::Var(0)],
        };
        let results = db.lookup(&goal);
        assert_eq!(results.len(), 3);

        // shape/1 should have 2 clauses
        let shape_id = db.interner.lookup("shape").unwrap();
        let goal = Term::Compound {
            functor: shape_id,
            args: vec![Term::Var(0)],
        };
        let results = db.lookup(&goal);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_lookup_specific_first_arg() {
        let db = build_db(
            "component(engine, piston). component(engine, crankshaft). component(brake, pad).",
        );
        let comp_id = db.interner.lookup("component").unwrap();
        let brake_id = db.interner.lookup("brake").unwrap();

        let goal = Term::Compound {
            functor: comp_id,
            args: vec![Term::Atom(brake_id), Term::Var(0)],
        };
        let results = db.lookup(&goal);
        assert_eq!(results.len(), 1);
    }
}
