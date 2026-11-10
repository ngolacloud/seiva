//! CI gate: the canonical `examples/**` modules obey the Clean-Architecture layer
//! dependency rule (ADR 0003), enforced here so a regression fails `cargo test`.
//!
//! The layer rule (`delonix_seiva::lint`) was a fitness function only ever run by
//! `seiva lint` over a project's `modules/` — never over the reference modules in
//! `examples/`, where every scaffolded module is copied from. So a forbidden
//! inter-layer edge added to an example (a `facade` reaching into `infrastructure`,
//! a `domain` pulling a DORM crate) passed CI green (SG2/SB-1). This test closes
//! that: it scans `examples/**` with the SAME rule and fails on any violation.

use std::path::Path;

use delonix_seiva::lint::{check_layers, scan_module_layers};

#[test]
fn examples_obey_the_layer_dependency_rule() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");

    // Top-level modules (accounts, notes, polls, …) plus the `shop/` grouping,
    // whose modules (catalog, orders, …) sit one level down.
    let mut modules = scan_module_layers(&examples).expect("scan examples/");
    modules.extend(scan_module_layers(&examples.join("shop")).expect("scan examples/shop/"));

    // Guard against a silently-empty scan (a moved directory, a path typo) that
    // would make this gate vacuously pass.
    assert!(
        modules.len() >= 3,
        "expected to discover the canonical example modules, found {}: {:?}",
        modules.len(),
        modules.iter().map(|m| &m.module).collect::<Vec<_>>()
    );

    let violations = check_layers(&modules);
    assert!(
        violations.is_empty(),
        "layer-rule violations in examples/ (a forbidden inter-layer dependency):\n{violations:#?}"
    );
}
