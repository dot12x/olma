use std::collections::HashMap;
use olma::resolver::Resolver;

fn graph(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
    pairs.iter()
        .map(|(k, vs)| (k.to_string(), vs.iter().map(|s| s.to_string()).collect()))
        .collect()
}

#[test]
fn linear_chain_orders_deps_first() {
    let g = graph(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);
    let order = Resolver::topo_sort_for_test(&g).unwrap();
    let pos = |x: &str| order.iter().position(|s| s == x).unwrap();
    assert!(pos("c") < pos("b"));
    assert!(pos("b") < pos("a"));
}

#[test]
fn diamond_dependency_appears_once() {
    let g = graph(&[
        ("a", &["b", "c"]),
        ("b", &["d"]),
        ("c", &["d"]),
        ("d", &[]),
    ]);
    let order = Resolver::topo_sort_for_test(&g).unwrap();
    assert_eq!(order.iter().filter(|s| s.as_str() == "d").count(), 1);
    let pos = |x: &str| order.iter().position(|s| s == x).unwrap();
    assert!(pos("d") < pos("b"));
    assert!(pos("d") < pos("c"));
    assert!(pos("b") < pos("a"));
    assert!(pos("c") < pos("a"));
}

#[test]
fn cycle_returns_error() {
    let g = graph(&[("a", &["b"]), ("b", &["a"])]);
    assert!(Resolver::topo_sort_for_test(&g).is_err());
}
