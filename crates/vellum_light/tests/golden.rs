//! The port against the Python reference: 35 golden cases (single lights,
//! the full pipeline at six game hours and two player strengths, and the
//! backdrop auto-colours), worst OKLab dE at most 0.01.

#[test]
fn matches_the_python_reference() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/golden");
    let worst = vellum_light::golden::run(dir);
    assert!(worst <= 0.01, "worst max dE {worst}");
}
