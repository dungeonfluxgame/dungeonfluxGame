//! The production metrics registry and its Prometheus text exposition (D20). One registry per
//! process; domain owners (finance, storage, cost, text and voice) register their series here
//! and no second exporter exists. Labels are bounded: every label value is a `&'static str`
//! chosen by program text (route templates and closed enums, never an id or player text), and a
//! family holds at most [`MAXIMUM_SERIES_PER_FAMILY`] series.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// Series one family may hold; a label value outside program text would exceed it.
pub const MAXIMUM_SERIES_PER_FAMILY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricsError {
    InvalidName,
    InvalidLabelName,
    DuplicateName,
    CounterNeedsTotalSuffix,
    BucketsNotAscending,
}

#[derive(Clone, Copy)]
enum Kind {
    Counter,
    Gauge,
    Histogram(&'static [f64]),
}

enum Cell {
    Counter(AtomicU64),
    /// The bits of an `f64`.
    Gauge(AtomicU64),
    Histogram {
        /// One count per bucket bound plus the overflow bucket; not cumulative.
        counts: Vec<AtomicU64>,
        sum_bits: AtomicU64,
    },
}

struct Family {
    name: &'static str,
    help: &'static str,
    kind: Kind,
    label_names: &'static [&'static str],
    series: Mutex<BTreeMap<Vec<&'static str>, Arc<Cell>>>,
    dropped: Dropped,
}

/// Counters for the observations a family refused, so nothing is lost silently.
#[derive(Clone)]
struct Dropped {
    series_bound: Arc<Cell>,
    label_count: Arc<Cell>,
    non_finite: Arc<Cell>,
}

impl Dropped {
    fn new() -> Self {
        let counter = || Arc::new(Cell::Counter(AtomicU64::new(0)));
        Self {
            series_bound: counter(),
            label_count: counter(),
            non_finite: counter(),
        }
    }

    fn count(cell: &Arc<Cell>) {
        if let Cell::Counter(count) = &**cell {
            count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Family {
    /// The series for these label values, created on first use; `None` for a wrong label
    /// count or when the family is at its series bound.
    fn cell(&self, values: &[&'static str]) -> Option<Arc<Cell>> {
        if values.len() != self.label_names.len() {
            Dropped::count(&self.dropped.label_count);
            return None;
        }
        let mut series = self
            .series
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = series.get(values) {
            return Some(Arc::clone(existing));
        }
        if series.len() >= MAXIMUM_SERIES_PER_FAMILY {
            Dropped::count(&self.dropped.series_bound);
            return None;
        }
        let cell = Arc::new(match self.kind {
            Kind::Counter => Cell::Counter(AtomicU64::new(0)),
            Kind::Gauge => Cell::Gauge(AtomicU64::new(0f64.to_bits())),
            Kind::Histogram(bounds) => Cell::Histogram {
                counts: (0..=bounds.len()).map(|_| AtomicU64::new(0)).collect(),
                sum_bits: AtomicU64::new(0f64.to_bits()),
            },
        });
        series.insert(values.to_vec(), Arc::clone(&cell));
        Some(cell)
    }
}

fn add_f64(bits: &AtomicU64, delta: f64) {
    let _previous = bits.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some((f64::from_bits(current) + delta).to_bits())
    });
}

/// A monotonically increasing count.
#[derive(Clone)]
pub struct Counter(Arc<Cell>);

impl Counter {
    pub fn add(&self, amount: u64) {
        if let Cell::Counter(count) = &*self.0 {
            count.fetch_add(amount, Ordering::Relaxed);
        }
    }

    pub fn inc(&self) {
        self.add(1);
    }
}

/// A value that goes up and down.
#[derive(Clone)]
pub struct Gauge(Arc<Cell>);

impl Gauge {
    pub fn set(&self, value: f64) {
        if let Cell::Gauge(bits) = &*self.0 {
            bits.store(value.to_bits(), Ordering::Relaxed);
        }
    }
}

/// Observations counted in fixed buckets.
#[derive(Clone)]
pub struct Histogram {
    cell: Arc<Cell>,
    bounds: &'static [f64],
    non_finite: Arc<Cell>,
}

impl Histogram {
    pub fn observe(&self, value: f64) {
        if !value.is_finite() {
            Dropped::count(&self.non_finite);
            return;
        }
        if let Cell::Histogram { counts, sum_bits } = &*self.cell {
            let slot = self
                .bounds
                .iter()
                .position(|bound| value <= *bound)
                .unwrap_or(self.bounds.len());
            counts[slot].fetch_add(1, Ordering::Relaxed);
            add_f64(sum_bits, value);
        }
    }
}

#[derive(Clone)]
pub struct CounterVec(Arc<Family>);

impl CounterVec {
    pub fn with(&self, values: &[&'static str]) -> Option<Counter> {
        self.0.cell(values).map(Counter)
    }
}

#[derive(Clone)]
pub struct GaugeVec(Arc<Family>);

impl GaugeVec {
    pub fn with(&self, values: &[&'static str]) -> Option<Gauge> {
        self.0.cell(values).map(Gauge)
    }
}

#[derive(Clone)]
pub struct HistogramVec(Arc<Family>, &'static [f64]);

impl HistogramVec {
    pub fn with(&self, values: &[&'static str]) -> Option<Histogram> {
        self.0.cell(values).map(|cell| Histogram {
            cell,
            bounds: self.1,
            non_finite: Arc::clone(&self.0.dropped.non_finite),
        })
    }
}

pub struct MetricsRegistry {
    families: Mutex<Vec<Arc<Family>>>,
    dropped: Dropped,
}

fn is_metric_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == ':')
        && characters.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

fn is_label_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("__")
}

impl MetricsRegistry {
    /// A registry that already carries `df_metrics_dropped_total{reason}`, which counts what a
    /// family refused: a series past its bound, a wrong label count, a non-finite observation.
    pub fn new() -> Self {
        let dropped = Dropped::new();
        let series = [
            (vec!["series_bound"], Arc::clone(&dropped.series_bound)),
            (vec!["label_count"], Arc::clone(&dropped.label_count)),
            (
                vec!["non_finite_observation"],
                Arc::clone(&dropped.non_finite),
            ),
        ];
        let family = Arc::new(Family {
            name: "df_metrics_dropped_total",
            help: "Observations a metric family refused instead of recording.",
            kind: Kind::Counter,
            label_names: &["reason"],
            series: Mutex::new(BTreeMap::from(series)),
            dropped: dropped.clone(),
        });
        Self {
            families: Mutex::new(vec![family]),
            dropped,
        }
    }

    fn register(
        &self,
        name: &'static str,
        help: &'static str,
        kind: Kind,
        label_names: &'static [&'static str],
    ) -> Result<Arc<Family>, MetricsError> {
        if !is_metric_name(name) {
            return Err(MetricsError::InvalidName);
        }
        if label_names
            .iter()
            .any(|label| !is_label_name(label) || *label == "le")
        {
            return Err(MetricsError::InvalidLabelName);
        }
        if matches!(kind, Kind::Counter) && !name.ends_with("_total") {
            return Err(MetricsError::CounterNeedsTotalSuffix);
        }
        if let Kind::Histogram(bounds) = kind
            && bounds.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(MetricsError::BucketsNotAscending);
        }
        let mut families = self
            .families
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if families.iter().any(|family| family.name == name) {
            return Err(MetricsError::DuplicateName);
        }
        let family = Arc::new(Family {
            name,
            help,
            kind,
            label_names,
            series: Mutex::new(BTreeMap::new()),
            dropped: self.dropped.clone(),
        });
        families.push(Arc::clone(&family));
        Ok(family)
    }

    pub fn counter(&self, name: &'static str, help: &'static str) -> Result<Counter, MetricsError> {
        self.counter_vec(name, help, &[])?
            .with(&[])
            .ok_or(MetricsError::InvalidLabelName)
    }

    pub fn counter_vec(
        &self,
        name: &'static str,
        help: &'static str,
        label_names: &'static [&'static str],
    ) -> Result<CounterVec, MetricsError> {
        self.register(name, help, Kind::Counter, label_names)
            .map(CounterVec)
    }

    pub fn gauge(&self, name: &'static str, help: &'static str) -> Result<Gauge, MetricsError> {
        self.gauge_vec(name, help, &[])?
            .with(&[])
            .ok_or(MetricsError::InvalidLabelName)
    }

    pub fn gauge_vec(
        &self,
        name: &'static str,
        help: &'static str,
        label_names: &'static [&'static str],
    ) -> Result<GaugeVec, MetricsError> {
        self.register(name, help, Kind::Gauge, label_names)
            .map(GaugeVec)
    }

    pub fn histogram(
        &self,
        name: &'static str,
        help: &'static str,
        bounds: &'static [f64],
    ) -> Result<Histogram, MetricsError> {
        self.histogram_vec(name, help, bounds, &[])?
            .with(&[])
            .ok_or(MetricsError::InvalidLabelName)
    }

    pub fn histogram_vec(
        &self,
        name: &'static str,
        help: &'static str,
        bounds: &'static [f64],
        label_names: &'static [&'static str],
    ) -> Result<HistogramVec, MetricsError> {
        self.register(name, help, Kind::Histogram(bounds), label_names)
            .map(|family| HistogramVec(family, bounds))
    }

    /// The Prometheus text exposition (format 0.0.4), families and series in sorted order.
    pub fn render(&self) -> String {
        let mut families: Vec<Arc<Family>> = self
            .families
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        families.sort_by_key(|family| family.name);
        let mut text = String::new();
        for family in families {
            render_family(&family, &mut text);
        }
        text
    }
}

fn render_family(family: &Family, text: &mut String) {
    let kind = match family.kind {
        Kind::Counter => "counter",
        Kind::Gauge => "gauge",
        Kind::Histogram(_) => "histogram",
    };
    let _ = writeln!(text, "# HELP {} {}", family.name, escape_help(family.help));
    let _ = writeln!(text, "# TYPE {} {kind}", family.name);
    let series = family
        .series
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (values, cell) in series.iter() {
        match (&**cell, family.kind) {
            (Cell::Counter(count), _) => {
                let labels = labels(family.label_names, values, None);
                let _ = writeln!(
                    text,
                    "{}{labels} {}",
                    family.name,
                    count.load(Ordering::Relaxed)
                );
            }
            (Cell::Gauge(bits), _) => {
                let labels = labels(family.label_names, values, None);
                let value = number(f64::from_bits(bits.load(Ordering::Relaxed)));
                let _ = writeln!(text, "{}{labels} {value}", family.name);
            }
            (Cell::Histogram { counts, sum_bits }, Kind::Histogram(bounds)) => {
                let mut cumulative = 0u64;
                for (slot, count) in counts.iter().enumerate() {
                    cumulative += count.load(Ordering::Relaxed);
                    let bound = bounds
                        .get(slot)
                        .map_or_else(|| "+Inf".to_string(), |b| number(*b));
                    let labels = labels(family.label_names, values, Some(&bound));
                    let _ = writeln!(text, "{}_bucket{labels} {cumulative}", family.name);
                }
                let plain = labels(family.label_names, values, None);
                let sum = number(f64::from_bits(sum_bits.load(Ordering::Relaxed)));
                let _ = writeln!(text, "{}_sum{plain} {sum}", family.name);
                let _ = writeln!(text, "{}_count{plain} {cumulative}", family.name);
            }
            (Cell::Histogram { .. }, _) => {}
        }
    }
}

fn labels(names: &[&str], values: &[&str], bucket_bound: Option<&str>) -> String {
    if names.is_empty() && bucket_bound.is_none() {
        return String::new();
    }
    let mut pairs: Vec<String> = names
        .iter()
        .zip(values)
        .map(|(name, value)| format!("{name}=\"{}\"", escape_label_value(value)))
        .collect();
    if let Some(bound) = bucket_bound {
        pairs.push(format!("le=\"{bound}\""));
    }
    format!("{{{}}}", pairs.join(","))
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn escape_help(help: &str) -> String {
    help.replace('\\', "\\\\").replace('\n', "\\n")
}

fn number(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value.is_infinite() {
        if value > 0.0 { "+Inf" } else { "-Inf" }.to_string()
    } else {
        value.to_string()
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}
