//! The charge-site guard of the cost engine: every `reserve` / `charge` call in `df-tools` must meter
//! its dollars through `SpendLedger::begin_call` or be a pinned, tracked backlog item. The guard
//! reads the sources with `syn`; the fixtures below pin what it counts as a charge and as a metering.

use std::{collections::BTreeSet, path::PathBuf};

/// The charge sites that do not meter dollars yet, by file and first argument: each is an item of
/// the cost engine backlog in `planning/issue-triage.md`.
pub(super) const UNMETERED_CHARGES: [(&str, &str, &str); 12] = [
    ("gameplay/character_clips.rs", "Unit::VideoClip", "C-B1"),
    (
        "gameplay/creator_classification.rs",
        "Unit::CreatorClassificationCall",
        "C-B5",
    ),
    ("gameplay/home_video.rs", "Unit::VideoClip", "C-B1"),
    (
        "gameplay/imagery/figure_masks.rs",
        "Unit::SegmentCall",
        "C-B1",
    ),
    (
        "gameplay/memory/embedder.rs",
        "Unit::EmbeddingToken",
        "C-B1",
    ),
    (
        "gameplay/place_music/resource.rs",
        "Unit::MusicCall",
        "C-B1",
    ),
    (
        "gameplay/place_music/resource.rs",
        "Unit::MusicCall",
        "C-B1",
    ),
    (
        "gameplay/plate_segments/service.rs",
        "Unit::SegmentCall",
        "C-B1",
    ),
    (
        "gameplay/plate_segments/service.rs",
        "Unit::TextToken",
        "C-B2",
    ),
    ("gameplay/speech_input.rs", "Unit::SttSecond", "C-B1"),
    ("gameplay/speech_input.rs", "seconds", "C-B1"),
    ("gameplay/splat_world.rs", "Unit::SplatCredit", "C-B1"),
];

/// One `reserve` or `charge` call, as the guard sees it.
#[derive(Debug, Eq, PartialEq)]
struct ChargeSite {
    /// The charged argument (`Unit::MusicCall`, `IMAGE_UNIT`, `unit`, `&call`; `<expr>` for any
    /// other expression, `<macro>` for a call found only in a macro's tokens).
    argument: String,
    /// The enclosing function: `Type::name` for a method of an impl or trait, `name` for a free
    /// function, empty outside any function.
    function: String,
    metered: bool,
}

/// What the guard records inside one scope, in source order.
enum GuardEvent {
    Charge {
        order: usize,
        argument: String,
        function: String,
        /// A `self.charge(..)` / `Self::charge(..)` of a method that returns a `BilledCall`.
        credited: bool,
    },
    BeginCall,
}

/// The charged argument of a call, as text.
fn argument_label(expression: &syn::Expr) -> String {
    match expression {
        syn::Expr::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        syn::Expr::Reference(reference) => format!("&{}", argument_label(&reference.expr)),
        _ => "<expr>".to_owned(),
    }
}

fn is_self(expression: &syn::Expr) -> bool {
    matches!(expression, syn::Expr::Path(path) if path.path.is_ident("self"))
}

/// Whether a `cfg` predicate holds only when tests are compiled: `test`; `all(..)` with such a
/// term; `any(..)` of only such terms. `not(..)` and every other predicate compile outside tests.
fn test_only(predicate: &syn::Meta) -> bool {
    let syn::Meta::List(list) = predicate else {
        return matches!(predicate, syn::Meta::Path(path) if path.is_ident("test"));
    };
    let all = list.path.is_ident("all");
    if !all && !list.path.is_ident("any") {
        return false;
    }
    let Ok(terms) = list.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    ) else {
        return false;
    };
    if all {
        terms.iter().any(test_only)
    } else {
        !terms.is_empty() && terms.iter().all(test_only)
    }
}

fn is_cfg_test(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Meta>()
                .is_ok_and(|predicate| test_only(&predicate))
    })
}

fn returns_billed_call(signature: &syn::Signature) -> bool {
    match &signature.output {
        syn::ReturnType::Type(_, returned) => matches!(
            returned.as_ref(),
            syn::Type::Path(path)
                if path.path.segments.last().is_some_and(|last| last.ident == "BilledCall")
        ),
        syn::ReturnType::Default => false,
    }
}

/// The last path segment of an impl's self type (`Kit` for `impl Kit`, `impl Trait for Kit`).
fn type_label(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default(),
        syn::Type::Reference(reference) => type_label(&reference.elem),
        _ => String::new(),
    }
}

fn function_label(owner: Option<&String>, name: &syn::Ident) -> String {
    match owner {
        Some(owner) if !owner.is_empty() => format!("{owner}::{name}"),
        _ => name.to_string(),
    }
}

/// The `(owner type, function)` pairs of the file whose return type is a `BilledCall`; the owner
/// is the self type of the enclosing impl or the enclosing trait, empty for a free function.
#[derive(Default)]
struct BilledFunctions {
    owners: Vec<String>,
    found: BTreeSet<(String, String)>,
}

impl BilledFunctions {
    fn note(&mut self, signature: &syn::Signature) {
        if returns_billed_call(signature) {
            let owner = self.owners.last().cloned().unwrap_or_default();
            self.found.insert((owner, signature.ident.to_string()));
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for BilledFunctions {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.owners.push(type_label(&item.self_ty));
        syn::visit::visit_item_impl(self, item);
        self.owners.pop();
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.owners.push(item.ident.to_string());
        syn::visit::visit_item_trait(self, item);
        self.owners.pop();
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.note(&item.sig);
        self.owners.push(String::new());
        syn::visit::visit_item_fn(self, item);
        self.owners.pop();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.note(&item.sig);
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.note(&item.sig);
        syn::visit::visit_trait_item_fn(self, item);
    }
}

/// How a name in a macro's tokens was reached: after `.` (a method call) or after `::` (a path,
/// so a fully qualified call whose first argument is the receiver).
#[derive(Clone, Copy)]
enum Reach {
    Method,
    Path,
}

fn skip_token_tree(input: syn::parse::ParseStream) -> syn::Result<()> {
    input.step(|cursor| match cursor.token_tree() {
        Some((_, rest)) => Ok(((), rest)),
        None => Err(cursor.error("expected a token")),
    })
}

/// The comma-separated arguments of a call's tokens (nested groups count as one token; a comma
/// inside generic arguments is counted, which can only add a site, never hide one).
fn token_arguments(input: syn::parse::ParseStream) -> syn::Result<usize> {
    let mut arguments = 0;
    let mut open = false;
    while !input.is_empty() {
        if input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
            open = false;
        } else {
            skip_token_tree(input)?;
            if !open {
                arguments += 1;
                open = true;
            }
        }
    }
    Ok(arguments)
}

/// The `reserve` / `charge` calls in a macro's tokens, found by walking the token tree: a name
/// after `.` or `::`, an optional turbofish, then a parenthesised group with enough arguments.
/// Every group is walked, so a call inside a `select!` arm or a nested macro is found.
fn token_charges(input: syn::parse::ParseStream) -> syn::Result<usize> {
    use syn::ext::IdentExt;
    use syn::parse::discouraged::Speculative;
    let mut sites = 0;
    let mut reach = None;
    while !input.is_empty() {
        if input.peek(syn::token::Paren)
            || input.peek(syn::token::Brace)
            || input.peek(syn::token::Bracket)
        {
            let content;
            if input.peek(syn::token::Paren) {
                syn::parenthesized!(content in input);
            } else if input.peek(syn::token::Brace) {
                syn::braced!(content in input);
            } else {
                syn::bracketed!(content in input);
            }
            sites += token_charges(&content)?;
            reach = None;
        } else if input.peek(syn::Token![.]) && !input.peek(syn::Token![..]) {
            input.parse::<syn::Token![.]>()?;
            reach = Some(Reach::Method);
        } else if input.peek(syn::Token![::]) {
            input.parse::<syn::Token![::]>()?;
            reach = Some(Reach::Path);
        } else if input.peek(syn::Ident::peek_any) {
            let name = input.call(syn::Ident::parse_any)?;
            let reached = reach.take();
            let ahead = input.fork();
            if ahead.peek(syn::Token![::])
                && ahead.peek3(syn::Token![<])
                && syn::AngleBracketedGenericArguments::parse_turbofish(&ahead).is_ok()
            {
                input.advance_to(&ahead);
            }
            if let (Some(reached), true) = (reached, input.peek(syn::token::Paren)) {
                let needed = match (name.to_string().as_str(), reached) {
                    ("reserve", Reach::Method) => 2,
                    ("reserve", Reach::Path) => 3,
                    ("charge", Reach::Method) => 1,
                    ("charge", Reach::Path) => 2,
                    _ => continue,
                };
                let content;
                syn::parenthesized!(content in input);
                if token_arguments(&content.fork())? >= needed {
                    sites += 1;
                }
                sites += token_charges(&content)?;
            }
        } else {
            skip_token_tree(input)?;
            reach = None;
        }
    }
    Ok(sites)
}

struct ChargeVisitor<'a> {
    billed: &'a BTreeSet<(String, String)>,
    /// The enclosing impl self types, traits and free functions (empty owner), innermost last.
    owners: Vec<String>,
    functions: Vec<String>,
    blocks: Vec<Vec<GuardEvent>>,
    sites: Vec<(usize, ChargeSite)>,
    order: usize,
}

impl ChargeVisitor<'_> {
    fn record(&mut self, event: GuardEvent) {
        match self.blocks.last_mut() {
            Some(block) => block.push(event),
            None => self.resolve(vec![event]),
        }
    }

    fn charge(&mut self, argument: String, credited: bool) {
        self.order += 1;
        let order = self.order;
        let function = self.functions.last().cloned().unwrap_or_default();
        self.record(GuardEvent::Charge {
            order,
            argument,
            function,
            credited,
        });
    }

    /// Runs `visit` in a scope of its own: a `begin_call` outside it meters nothing inside it, and
    /// the other way round.
    fn scoped(&mut self, visit: impl FnOnce(&mut Self)) {
        self.blocks.push(Vec::new());
        visit(self);
        let events = self.blocks.pop().unwrap_or_default();
        self.resolve(events);
    }

    /// A charge is metered when a `begin_call` follows it in its own scope before the next
    /// charge, or when it is a credited `self.charge`.
    fn resolve(&mut self, events: Vec<GuardEvent>) {
        for (index, event) in events.iter().enumerate() {
            let GuardEvent::Charge {
                order,
                argument,
                function,
                credited,
            } = event
            else {
                continue;
            };
            let followed = events[index + 1..]
                .iter()
                .take_while(|later| !matches!(later, GuardEvent::Charge { .. }))
                .any(|later| matches!(later, GuardEvent::BeginCall));
            self.sites.push((
                *order,
                ChargeSite {
                    argument: argument.clone(),
                    function: function.clone(),
                    metered: *credited || followed,
                },
            ));
        }
    }

    /// A call named `name` with `arguments` (the receiver of a fully qualified call is not one).
    /// `reserve` takes a unit and an amount, `charge` at least one argument, so a container's
    /// `.reserve(n)` is not a site. `own` is a call on this type itself (`self.` or `Self::`).
    fn charged_call(&mut self, name: &str, arguments: &[&syn::Expr], own: bool) {
        match name {
            "reserve" | "charge" => {
                let needed = if name == "reserve" { 2 } else { 1 };
                if let Some(first) = arguments.first().filter(|_| arguments.len() >= needed) {
                    let owner = self.owners.last().cloned().unwrap_or_default();
                    let credited =
                        own && name == "charge" && self.billed.contains(&(owner, name.to_owned()));
                    self.charge(argument_label(first), credited);
                }
            }
            "begin_call" => self.record(GuardEvent::BeginCall),
            _ => {}
        }
    }

    /// Walks a function or method body under its label.
    fn in_function(&mut self, label: String, visit: impl FnOnce(&mut Self)) {
        self.functions.push(label);
        visit(self);
        self.functions.pop();
    }
}

impl<'ast> syn::visit::Visit<'ast> for ChargeVisitor<'_> {
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.scoped(|visitor| syn::visit::visit_block(visitor, block));
    }

    /// A match arm is one of several paths: a `begin_call` in another arm meters nothing here.
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.scoped(|visitor| syn::visit::visit_arm(visitor, arm));
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        self.scoped(|visitor| syn::visit::visit_expr_closure(visitor, closure));
    }

    /// The right operand of `&&` and `||` may not run.
    fn visit_expr_binary(&mut self, binary: &'ast syn::ExprBinary) {
        self.visit_expr(&binary.left);
        if matches!(binary.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
            self.scoped(|visitor| visitor.visit_expr(&binary.right));
        } else {
            self.visit_expr(&binary.right);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !is_cfg_test(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if !is_cfg_test(&item.attrs) {
            self.owners.push(type_label(&item.self_ty));
            syn::visit::visit_item_impl(self, item);
            self.owners.pop();
        }
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if !is_cfg_test(&item.attrs) {
            self.owners.push(item.ident.to_string());
            syn::visit::visit_item_trait(self, item);
            self.owners.pop();
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !is_cfg_test(&item.attrs) {
            self.owners.push(String::new());
            let label = function_label(None, &item.sig.ident);
            self.in_function(label, |visitor| syn::visit::visit_item_fn(visitor, item));
            self.owners.pop();
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !is_cfg_test(&item.attrs) {
            let label = function_label(self.owners.last(), &item.sig.ident);
            self.in_function(label, |visitor| {
                syn::visit::visit_impl_item_fn(visitor, item);
            });
        }
    }

    /// A trait's default body is code of the trait.
    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if !is_cfg_test(&item.attrs) {
            let label = function_label(self.owners.last(), &item.sig.ident);
            self.in_function(label, |visitor| {
                syn::visit::visit_trait_item_fn(visitor, item);
            });
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.visit_expr(&call.receiver);
        let arguments: Vec<&syn::Expr> = call.args.iter().collect();
        self.charged_call(
            &call.method.to_string(),
            &arguments,
            is_self(&call.receiver),
        );
        for argument in &call.args {
            self.visit_expr(argument);
        }
    }

    /// A fully qualified call (`SpendLedger::reserve(&ledger, unit, n)`, `Self::charge(self, c)`,
    /// `<SpendLedger>::reserve(..)`): its first argument is the receiver. A call through a bare
    /// one-segment name is not a ledger call: methods cannot be imported by name.
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        self.visit_expr(&call.func);
        if let syn::Expr::Path(path) = call.func.as_ref() {
            let segments = &path.path.segments;
            if let Some(last) = segments
                .last()
                .filter(|_| segments.len() > 1 || path.qself.is_some())
            {
                let own = path.qself.is_none()
                    && segments.len() == 2
                    && segments.first().is_some_and(|first| first.ident == "Self");
                let arguments: Vec<&syn::Expr> = call.args.iter().skip(1).collect();
                self.charged_call(&last.ident.to_string(), &arguments, own);
            }
        }
        for argument in &call.args {
            self.visit_expr(argument);
        }
    }

    /// A macro whose tokens are expressions (`assert!`, `format!`) is read as them; any other
    /// macro is walked by its tokens, and each charge found there is an unmetered site, so a
    /// macro cannot hide a charge. A macro that cannot be read counts as one such site.
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        match mac.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            Ok(expressions) => {
                for expression in &expressions {
                    self.visit_expr(expression);
                }
            }
            Err(_) => {
                for _ in 0..mac.parse_body_with(token_charges).unwrap_or(1) {
                    self.charge("<macro>".to_owned(), false);
                }
            }
        }
    }
}

/// Every charge site of one source file, in source order, read by `syn`: comments and literals
/// are not code, so they neither hide a charge nor meter one. A site is a `reserve` call with a
/// unit and an amount, or a `charge` call with an argument, whatever the unit (a unit constant,
/// another constant, a variable), in method, turbofish or fully qualified form; a container's
/// `table_log.reserve(n)` is not. A site is metered only when a `begin_call` follows it in the same
/// scope (block, match arm, closure, or the right operand of `&&` / `||`) before the next
/// charge, or when it is `self.charge(..)` / `Self::charge(..)` of a method of the same impl or
/// trait that returns a `BilledCall` (imagery's `charge`, whose own body is a checked site).
/// Code compiled only for tests (`#[cfg(test)]` and `all(test, ..)` items) is skipped.
fn charge_sites(source: &str) -> syn::Result<Vec<ChargeSite>> {
    let file = syn::parse_file(source)?;
    let mut billed = BilledFunctions::default();
    syn::visit::Visit::visit_file(&mut billed, &file);
    let mut visitor = ChargeVisitor {
        billed: &billed.found,
        owners: Vec::new(),
        functions: Vec::new(),
        blocks: Vec::new(),
        sites: Vec::new(),
        order: 0,
    };
    syn::visit::Visit::visit_file(&mut visitor, &file);
    visitor.sites.sort_by_key(|(order, _)| *order);
    Ok(visitor.sites.into_iter().map(|(_, site)| site).collect())
}

/// One charge fixture: the source, then the sites the guard must report, in order.
fn guard_case(source: &str, expected: &[(&str, bool)]) {
    let found: Vec<(String, bool)> = charge_sites(source)
        .expect("the fixture parses as Rust")
        .into_iter()
        .map(|site| (site.argument, site.metered))
        .collect();
    let expected: Vec<(String, bool)> = expected
        .iter()
        .map(|(argument, metered)| ((*argument).to_owned(), *metered))
        .collect();
    assert_eq!(found, expected, "{source}");
}

/// The functions that are the metering itself or forward to it, by file and function (as
/// `ChargeSite::function` names them). The kit's generic `charge` takes its unit from its caller,
/// whose call is the site that is checked. The ledger (`resources.rs`) and the cost book
/// (`cost.rs`) have no call site outside their `#[cfg(test)]` code, so no whole file is exempt.
const CHARGE_DEFINITIONS: [(&str, &str); 1] = [("gameplay/resource_adapters.rs", "Kit::charge")];

#[test]
fn every_spend_charge_meters_its_dollars_or_is_a_tracked_backlog_item() {
    fn visit(directory: &std::path::Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).expect("sources") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();
    let mut metered = Vec::new();
    let mut unmetered = Vec::new();
    let mut defined = BTreeSet::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        let name = relative.rsplit('/').next().unwrap_or_default();
        if name.ends_with("_tests.rs") || name == "tests.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source");
        // A file that says neither word has no charge site: it is not parsed.
        if !text.contains("reserve") && !text.contains("charge") {
            continue;
        }
        let sites = charge_sites(&text)
            .unwrap_or_else(|error| panic!("{relative} does not parse as Rust: {error}"));
        for site in sites {
            if CHARGE_DEFINITIONS.contains(&(relative.as_str(), site.function.as_str())) {
                defined.insert((relative.clone(), site.function));
                continue;
            }
            let entry = (relative.clone(), site.argument);
            if site.metered {
                metered.push(entry);
            } else {
                unmetered.push(entry);
            }
        }
    }
    assert_eq!(
        defined.len(),
        CHARGE_DEFINITIONS.len(),
        "every exempt metering function still has a charge in it: {defined:?}"
    );
    let expected: Vec<(String, String)> = UNMETERED_CHARGES
        .iter()
        .map(|(file, argument, _)| ((*file).to_owned(), (*argument).to_owned()))
        .collect();
    assert_eq!(
        unmetered, expected,
        "a charge with no begin_call after it must meter its dollars or be listed with its \
         backlog item (a container's .reserve(n) is no site: it has one argument)"
    );
    for (file, argument) in [
        ("gameplay/character_clips.rs", "Unit::ImageCall"),
        ("gameplay/imagery.rs", "IMAGE_UNIT"),
        ("gameplay/imagery.rs", "call"),
        ("gameplay/storyteller.rs", "Unit::TextToken"),
        ("gameplay/voice.rs", "Unit::VoiceCharacter"),
    ] {
        assert!(
            metered.contains(&(file.to_owned(), argument.to_owned())),
            "{file} {argument} is metered: {metered:?}"
        );
    }
}

/// The function a site is found in is named as `Type::name` (an impl or a trait), `name` (a free
/// function) or empty (outside any function).
#[test]
fn a_charge_site_names_its_function() {
    let sites = charge_sites(
        "fn free(ledger: &Ledger) { ledger.reserve(Unit::MusicCall, 1).commit(1); }
        impl Kit {
            fn charge(&self, unit: Unit) { self.ledger.reserve(unit, 1).commit(1); }
        }
        trait Probe {
            fn probe(&self) { self.ledger.reserve(Unit::VideoClip, 1).commit(1); }
        }
        const SPENT: u64 = ledger().reserve(Unit::SegmentCall, 1).amount();",
    )
    .expect("the fixture parses as Rust");
    let found: Vec<(&str, &str)> = sites
        .iter()
        .map(|site| (site.function.as_str(), site.argument.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("free", "Unit::MusicCall"),
            ("Kit::charge", "unit"),
            ("Probe::probe", "Unit::VideoClip"),
            ("", "Unit::SegmentCall"),
        ]
    );
}

/// A file that does not parse names itself in the failure the scan reports.
#[test]
fn a_source_that_does_not_parse_is_an_error_not_an_empty_scan() {
    assert!(charge_sites("fn broken( {").is_err());
}

#[test]
fn the_charge_guard_reads_wrapped_calls_variables_and_one_metering_per_charge() {
    let source = "fn fixture() {
        // ledger.reserve(Unit::ImageCall, 1) in a comment is not a call
        let reservation = ledger
            .reserve(
                Unit::MusicCall,
                u64::from(PROVIDER_CALLS),
            );
        let row = table_log.reserve();
        self.hub.ledger().reserve(unit, amount).commit(amount);
        self.kit
            .get()
            .charge(Unit::TextToken, reserved);
        let billed = ledger.begin_call(spec, &[]);
    }";
    guard_case(
        source,
        &[
            ("Unit::MusicCall", false),
            ("unit", false),
            ("Unit::TextToken", true),
        ],
    );
    let wrapper = "impl Engine {
        fn run(&self) {
            let billed = self.charge(call);
        }
        fn charge(&self, call: &CallSpec) -> super::cost::BilledCall {
            self.ledger.reserve(IMAGE_UNIT, 1).commit(1);
            self.ledger.begin_call(call.clone(), &[])
        }
    }";
    guard_case(wrapper, &[("call", true), ("IMAGE_UNIT", true)]);
}

#[test]
fn a_comment_or_literal_mentioning_begin_call_meters_nothing() {
    guard_case(
        r##"fn a(ledger: &Ledger) {
            ledger.reserve(Unit::MusicCall, 1).commit(1); // metered by begin_call( later
            /* begin_call( */
            let text = "begin_call(";
            let raw = r#"begin_call("#;
            let quote = '(';
        }"##,
        &[("Unit::MusicCall", false)],
    );
}

#[test]
fn the_metering_search_ends_at_the_block_that_holds_the_charge() {
    guard_case(
        "fn a(ledger: &Ledger) {
            ledger.reserve(Unit::SegmentCall, 1).commit(1);
        }
        fn b(ledger: &Ledger) {
            let billed = ledger.begin_call(spec(), &[]);
        }",
        &[("Unit::SegmentCall", false)],
    );
}

/// A match arm is one of several paths: a `begin_call` in another arm meters nothing here, braced
/// or not.
#[test]
fn a_begin_call_in_another_match_arm_meters_nothing() {
    guard_case(
        "fn a(ledger: &Ledger, kind: Kind) {
            match kind {
                Kind::Music => ledger.reserve(Unit::MusicCall, 1).commit(1),
                Kind::Video => ledger.begin_call(spec(), &[]),
            }
        }",
        &[("Unit::MusicCall", false)],
    );
    guard_case(
        "fn a(ledger: &Ledger, kind: Kind) {
            match kind {
                Kind::Music => {
                    ledger.reserve(Unit::MusicCall, 1).commit(1);
                    let billed = ledger.begin_call(spec(), &[]);
                }
                Kind::Video => {}
            }
        }",
        &[("Unit::MusicCall", true)],
    );
}

/// A closure may never run, or run elsewhere: its body and the code around it do not meter each
/// other.
#[test]
fn a_begin_call_inside_or_outside_a_closure_meters_nothing_across_it() {
    guard_case(
        "fn a(ledger: &Ledger) {
            let send = || ledger.reserve(Unit::SegmentCall, 1).commit(1);
            let billed = ledger.begin_call(spec(), &[]);
        }",
        &[("Unit::SegmentCall", false)],
    );
    guard_case(
        "fn a(ledger: &Ledger) {
            ledger.reserve(Unit::SegmentCall, 1).commit(1);
            let meter = || ledger.begin_call(spec(), &[]);
        }",
        &[("Unit::SegmentCall", false)],
    );
}

/// The right operand of `&&` may not run.
#[test]
fn a_begin_call_in_the_right_operand_of_and_meters_nothing() {
    guard_case(
        "fn a(ledger: &Ledger, ready: bool) {
            ledger.reserve(Unit::VideoClip, 1).commit(1);
            let metered = ready && ledger.begin_call(spec(), &[]).is_held();
        }",
        &[("Unit::VideoClip", false)],
    );
}

/// The right operand of `||` may not run: a charge there is not metered by a later `begin_call`.
#[test]
fn a_charge_in_the_right_operand_of_or_is_not_metered_by_a_later_begin_call() {
    guard_case(
        "fn a(ledger: &Ledger, ready: bool) {
            let held = ready || ledger.reserve(Unit::VideoClip, 1).is_held();
            let billed = ledger.begin_call(spec(), &[]);
        }",
        &[("Unit::VideoClip", false)],
    );
}

#[test]
fn fully_qualified_and_turbofish_charges_are_charge_sites() {
    guard_case(
        "fn a(ledger: &Ledger, kit: &Kit) {
            SpendLedger::reserve(&ledger, Unit::MusicCall, 1).commit(1);
            kit.charge::<u64>(Unit::TextToken, 9);
            <SpendLedger>::reserve(&ledger, Unit::VideoClip, 1).commit(1);
            SpendLedger::reserve::<u64>(&ledger, Unit::SegmentCall, 1).commit(1);
        }",
        &[
            ("Unit::MusicCall", false),
            ("Unit::TextToken", false),
            ("Unit::VideoClip", false),
            ("Unit::SegmentCall", false),
        ],
    );
}

/// `reserve` takes a unit and an amount: a container's `reserve(n)` is not a charge, a fully
/// qualified call does not count its receiver, and a bare one-segment call is not a ledger call.
#[test]
fn a_reserve_needs_a_unit_and_an_amount() {
    guard_case(
        "fn a(ledger: &Ledger, mut rows: Vec<u8>) {
            rows.reserve(64);
            Vec::reserve(&mut rows, 64);
            reserve(Unit::MusicCall, 1);
            ledger.reserve(Unit::MusicCall, 1).commit(1);
            SpendLedger::reserve(&ledger, Unit::VideoClip, 1).commit(1);
        }",
        &[("Unit::MusicCall", false), ("Unit::VideoClip", false)],
    );
}

/// A billed `charge` is credited by the impl that defines it, not by a method of the same name in
/// another impl of the file.
#[test]
fn a_billed_charge_is_credited_only_inside_its_own_impl() {
    guard_case(
        "impl Billing {
            fn charge(&self, call: &CallSpec) -> BilledCall {
                self.ledger.begin_call(call.clone(), &[])
            }
            fn run(&self) { let billed = self.charge(&call); }
        }
        impl Plain {
            fn charge(&self, call: &CallSpec) { let _ = call; }
            fn run(&self) { self.charge(&call); Self::charge(self, &call); }
        }
        impl Billing {
            fn again(&self) { Self::charge(self, &call); }
        }",
        &[
            ("&call", true),
            ("&call", false),
            ("&call", false),
            ("&call", true),
        ],
    );
}

/// Only this type's own `BilledCall`-returning `charge` is credited, and only its return type
/// counts: another receiver's charge stays a charge.
#[test]
fn only_a_self_charge_returning_a_billed_call_is_credited() {
    guard_case(
        "impl Engine {
            fn charge(&self, call: &CallSpec) -> BilledCall {
                self.ledger.reserve(IMAGE_UNIT, 1).commit(1);
                self.ledger.begin_call(call.clone(), &[])
            }
            fn locate(&self) {
                self.kit.charge(Unit::TextToken, 7);
                let billed = self.charge(&call);
            }
        }",
        &[
            ("IMAGE_UNIT", true),
            ("Unit::TextToken", false),
            ("&call", true),
        ],
    );
    guard_case(
        "impl Engine {
            fn charge(&self, call: BilledCall) {}
            fn run(&self) {
                self.charge(Unit::ImageCall);
            }
        }",
        &[("Unit::ImageCall", false)],
    );
}

#[test]
fn a_charge_in_a_macro_is_found_by_its_tokens() {
    guard_case(
        "fn fixture() {
            assert!(ledger.reserve(Unit::MusicCall, 1).is_held());
            tokio::select! { _ = tick => kit.charge(Unit::VideoClip, 1), }
        }",
        &[("Unit::MusicCall", false), ("<macro>", false)],
    );
    // A turbofish, a fully qualified call and a nested group inside a macro that is not a list
    // of expressions; a container's reserve(n) is no site.
    guard_case(
        "fn fixture() {
            tokio::select! {
                _ = tick => { rows.reserve(8); }
                _ = done => { kit.charge::<u64>(Unit::TextToken, 9); }
                _ = other => { (SpendLedger::reserve(&ledger, Unit::SegmentCall, 2)); }
                _ = last => { wrapper!([ledger.reserve(Unit::MusicCall, 1)]); }
            }
        }",
        &[("<macro>", false), ("<macro>", false), ("<macro>", false)],
    );
}

#[test]
fn code_compiled_only_for_tests_is_not_scanned() {
    guard_case(
        "fn fixture() {
            ledger.reserve(Unit::MusicCall, 1).commit(1);
        }
        #[cfg(test)]
        mod tests {
            fn helper() { ledger.reserve(Unit::ImageCall, 1).commit(1); }
        }
        #[cfg(test)]
        fn helper() { ledger.reserve(Unit::ImageCall, 1).commit(1); }
        impl Engine {
            #[cfg(test)]
            fn probe(&self) { self.ledger.reserve(Unit::VideoClip, 1).commit(1); }
        }
        #[cfg(test)]
        impl Engine {
            fn probe(&self) { self.ledger.reserve(Unit::VideoClip, 2).commit(1); }
        }
        trait Probe {
            #[cfg(test)]
            fn probe(&self) { self.ledger.reserve(Unit::VideoClip, 3).commit(1); }
        }
        #[cfg(test)]
        trait Other {
            fn probe(&self) { self.ledger.reserve(Unit::VideoClip, 4).commit(1); }
        }
        #[cfg(all(test, feature = \"slow\"))]
        mod slow {
            fn helper() { ledger.reserve(Unit::SegmentCall, 1).commit(1); }
        }",
        &[("Unit::MusicCall", false)],
    );
}

#[test]
fn code_that_also_compiles_outside_tests_is_scanned() {
    guard_case(
        "#[cfg(any(test, unix))]
        fn both() { ledger.reserve(Unit::MusicCall, 1).commit(1); }
        #[cfg(not(test))]
        fn production() { ledger.reserve(Unit::VideoClip, 1).commit(1); }
        #[cfg(feature = \"slow\")]
        impl Engine {
            fn probe(&self) { self.ledger.reserve(Unit::SegmentCall, 1).commit(1); }
        }
        trait Probe {
            fn probe(&self) { self.ledger.reserve(Unit::ImageCall, 1).commit(1); }
        }",
        &[
            ("Unit::MusicCall", false),
            ("Unit::VideoClip", false),
            ("Unit::SegmentCall", false),
            ("Unit::ImageCall", false),
        ],
    );
}
