use crate::metrics::{MAXIMUM_SERIES_PER_FAMILY, MetricsError, MetricsRegistry};

const BUCKETS: &[f64] = &[1.0, 5.0, 25.0];

#[test]
fn a_counter_renders_help_type_and_its_value() {
    let registry = MetricsRegistry::new();
    let renewals = registry
        .counter(
            "df_lease_renew_failures_total",
            "Lease renewals that failed.",
        )
        .expect("counter");
    renewals.inc();
    renewals.add(2);
    assert_eq!(
        without_the_dropped_family(&registry.render()),
        "# HELP df_lease_renew_failures_total Lease renewals that failed.\n\
         # TYPE df_lease_renew_failures_total counter\n\
         df_lease_renew_failures_total 3\n"
    );
}

#[test]
fn labelled_series_render_sorted_with_escaped_values() {
    let registry = MetricsRegistry::new();
    let lookups = registry
        .counter_vec(
            "df_edge_route_lookups_total",
            "Route lookups.",
            &["outcome"],
        )
        .expect("vec");
    lookups.with(&["found"]).expect("found").add(4);
    lookups.with(&["a\"b\\c\nd"]).expect("odd").inc();
    let text = registry.render();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.contains("df_metrics_dropped_total"))
        .collect();
    assert_eq!(
        lines,
        [
            "df_edge_route_lookups_total{outcome=\"a\\\"b\\\\c\\nd\"} 1",
            "df_edge_route_lookups_total{outcome=\"found\"} 4",
        ]
    );
}

#[test]
fn a_gauge_holds_the_last_value_set() {
    let registry = MetricsRegistry::new();
    let connected = registry
        .gauge("df_players_connected", "Players connected.")
        .expect("gauge");
    connected.set(3.0);
    connected.set(2.5);
    assert!(registry.render().contains("df_players_connected 2.5\n"));
}

#[test]
fn a_histogram_renders_cumulative_buckets_sum_and_count() {
    let registry = MetricsRegistry::new();
    let reduce = registry
        .histogram("df_owner_reduce_ms", "Reduce time.", BUCKETS)
        .expect("histogram");
    for value in [0.5, 3.0, 3.0, 100.0] {
        reduce.observe(value);
    }
    let text = registry.render();
    for expected in [
        "df_owner_reduce_ms_bucket{le=\"1\"} 1",
        "df_owner_reduce_ms_bucket{le=\"5\"} 3",
        "df_owner_reduce_ms_bucket{le=\"25\"} 3",
        "df_owner_reduce_ms_bucket{le=\"+Inf\"} 4",
        "df_owner_reduce_ms_sum 106.5",
        "df_owner_reduce_ms_count 4",
    ] {
        assert!(
            text.lines().any(|line| line == expected),
            "missing `{expected}` in\n{text}"
        );
    }
}

#[test]
fn registration_refuses_bad_names_duplicates_and_unsorted_buckets() {
    let registry = MetricsRegistry::new();
    assert_eq!(
        registry.gauge("1bad", "x").err(),
        Some(MetricsError::InvalidName)
    );
    assert_eq!(
        registry.counter("df_things", "x").err(),
        Some(MetricsError::CounterNeedsTotalSuffix)
    );
    assert_eq!(
        registry.gauge_vec("df_ok", "x", &["le"]).err(),
        Some(MetricsError::InvalidLabelName)
    );
    registry.gauge("df_once", "x").expect("first");
    assert_eq!(
        registry.gauge("df_once", "x").err(),
        Some(MetricsError::DuplicateName)
    );
    assert_eq!(
        registry.histogram("df_sorted", "x", &[5.0, 1.0]).err(),
        Some(MetricsError::BucketsNotAscending)
    );
}

#[test]
fn a_family_refuses_a_wrong_label_count_and_stops_at_its_series_bound() {
    const NAMES: [&str; 70] = [
        "v00", "v01", "v02", "v03", "v04", "v05", "v06", "v07", "v08", "v09", "v10", "v11", "v12",
        "v13", "v14", "v15", "v16", "v17", "v18", "v19", "v20", "v21", "v22", "v23", "v24", "v25",
        "v26", "v27", "v28", "v29", "v30", "v31", "v32", "v33", "v34", "v35", "v36", "v37", "v38",
        "v39", "v40", "v41", "v42", "v43", "v44", "v45", "v46", "v47", "v48", "v49", "v50", "v51",
        "v52", "v53", "v54", "v55", "v56", "v57", "v58", "v59", "v60", "v61", "v62", "v63", "v64",
        "v65", "v66", "v67", "v68", "v69",
    ];
    let registry = MetricsRegistry::new();
    let routes = registry
        .counter_vec("df_routes_total", "Routes.", &["route"])
        .expect("vec");
    assert!(
        routes.with(&[]).is_none(),
        "a missing label value is refused"
    );
    assert!(
        routes.with(&["a", "b"]).is_none(),
        "an extra label value is refused"
    );
    let accepted = NAMES
        .iter()
        .filter(|name| routes.with(&[name]).is_some())
        .count();
    assert_eq!(accepted, MAXIMUM_SERIES_PER_FAMILY);
    assert!(
        routes.with(&[NAMES[0]]).is_some(),
        "an existing series stays reachable"
    );
}

/// The exposition without the registry's own `df_metrics_dropped_total` family.
fn without_the_dropped_family(text: &str) -> String {
    text.lines()
        .filter(|line| !line.contains("df_metrics_dropped_total"))
        .map(|line| format!("{line}\n"))
        .collect()
}

#[test]
fn refusals_are_counted_and_a_non_finite_observation_never_poisons_a_histogram() {
    let registry = MetricsRegistry::new();
    let routes = registry
        .counter_vec("df_routes_total", "Routes.", &["route"])
        .expect("vec");
    assert!(routes.with(&["a", "b"]).is_none());
    let latency = registry
        .histogram("df_latency_ms", "Latency.", BUCKETS)
        .expect("histogram");
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 2.0] {
        latency.observe(value);
    }
    const NAMES: [&str; 65] = [
        "n00", "n01", "n02", "n03", "n04", "n05", "n06", "n07", "n08", "n09", "n10", "n11", "n12",
        "n13", "n14", "n15", "n16", "n17", "n18", "n19", "n20", "n21", "n22", "n23", "n24", "n25",
        "n26", "n27", "n28", "n29", "n30", "n31", "n32", "n33", "n34", "n35", "n36", "n37", "n38",
        "n39", "n40", "n41", "n42", "n43", "n44", "n45", "n46", "n47", "n48", "n49", "n50", "n51",
        "n52", "n53", "n54", "n55", "n56", "n57", "n58", "n59", "n60", "n61", "n62", "n63", "n64",
    ];
    for name in NAMES {
        let _ = routes.with(&[name]);
    }
    let text = registry.render();
    for expected in [
        "df_metrics_dropped_total{reason=\"label_count\"} 1",
        "df_metrics_dropped_total{reason=\"non_finite_observation\"} 3",
        "df_metrics_dropped_total{reason=\"series_bound\"} 1",
        "df_latency_ms_sum 2",
        "df_latency_ms_count 1",
    ] {
        assert!(
            text.lines().any(|line| line == expected),
            "missing `{expected}` in\n{text}"
        );
    }
}
