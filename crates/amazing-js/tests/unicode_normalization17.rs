#[path = "support/unicode_normalization17.rs"]
mod corpus;
use amazing_js::{Interpreter, interp::ExecutionBackend};

#[test]
fn official_unicode17_all_twenty_row_invariants_use_installed_normalizer() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let result = corpus::run_rows(&mut Interpreter::new(), backend);
        assert_eq!(result.failed_checks, 0, "{backend:?}: {}", result.diagnostics.join("\n"));
        assert_eq!(result.row_statuses.len(), corpus::EXPECTED_ROWS);
    }
}

#[test]
fn official_unicode17_unlisted_points_and_surrogates_are_inert_across_starter_barriers() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (count, failures) = corpus::run_unlisted(&mut Interpreter::new(), backend);
        assert!(count > 1_000_000);
        assert!(failures.is_empty(), "{backend:?}: {}", failures.join("\n"));
    }
}
