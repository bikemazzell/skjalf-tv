//! Browser state: filtering, search, selection restore, and caching.

use skjalf_core::browse::{BrowserFocus, BrowserState, CatalogIndex, CategorySel, SortMode};

const GROUPS: usize = 10;

struct Item {
    key: String,
    name: String,
    group: String,
}

fn catalog(count: usize) -> Vec<Item> {
    (0..count)
        .map(|n| Item {
            key: format!("k{n}"),
            name: if n % 1000 == 7 {
                format!("Item {n:06} NeedleCase")
            } else {
                format!("Item {n:06}")
            },
            group: format!("g{}", n % GROUPS),
        })
        .collect()
}

fn index_of(items: &[Item]) -> CatalogIndex {
    CatalogIndex::build(
        items
            .iter()
            .map(|item| (item.key.as_str(), item.name.as_str(), item.group.as_str())),
    )
}

fn refresh(state: &mut BrowserState, index: &CatalogIndex, favorites: &[&str]) -> bool {
    state.refresh(index, 1, 1, || {
        favorites.iter().map(|key| key.to_string()).collect()
    })
}

#[test]
fn large_catalog_filters_by_category_and_query() {
    let items = catalog(200_000);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    assert!(refresh(&mut state, &index, &[]));
    assert_eq!(state.filtered.len(), 200_000);

    state.query = "  needlecase ".into();
    assert!(refresh(&mut state, &index, &[]));
    let expected: Vec<usize> = (0..200_000).filter(|n| n % 1000 == 7).collect();
    assert_eq!(
        state.filtered, expected,
        "case-insensitive, trimmed, whole catalog"
    );

    state.set_category(CategorySel::Group("g7".into()));
    assert!(refresh(&mut state, &index, &[]));
    let expected: Vec<usize> = (0..200_000)
        .filter(|n| n % 1000 == 7 && n % GROUPS == 7)
        .collect();
    assert_eq!(state.filtered, expected);

    state.query.clear();
    state.set_category(CategorySel::Group("g3".into()));
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered.len(), 20_000);
    assert!(state.filtered.iter().all(|n| n % GROUPS == 3));
}

#[test]
fn unchanged_key_does_not_recompute() {
    let items = catalog(100);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    assert!(refresh(&mut state, &index, &[]));
    assert!(!refresh(&mut state, &index, &[]));
    state.query = "item 0000".into();
    assert!(refresh(&mut state, &index, &[]));
    state.query = "ITEM 0000  ".into();
    assert!(
        !refresh(&mut state, &index, &[]),
        "normalized query is unchanged"
    );
    assert!(
        state.refresh(&index, 2, 1, Vec::new),
        "new catalog generation"
    );
    assert!(
        state.refresh(&index, 2, 2, Vec::new),
        "new favorites generation"
    );
}

#[test]
fn favorites_closure_runs_only_when_favorites_need_a_recompute() {
    let items = catalog(10);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    state.refresh(&index, 1, 1, || panic!("All must not read favorites"));
    state.set_category(CategorySel::Favorites);
    let mut calls = 0;
    state.refresh(&index, 1, 1, || {
        calls += 1;
        vec!["k4".into()]
    });
    assert_eq!(calls, 1);
    assert!(!state.refresh(&index, 1, 1, || panic!("cached")));
}

#[test]
fn favorites_keep_their_order_skip_missing_keys_and_honor_the_query() {
    let items = catalog(20);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::Favorites);
    refresh(&mut state, &index, &["k5", "k1", "gone", "k12"]);
    assert_eq!(state.filtered, [5, 1, 12]);
    state.query = "000012".into();
    refresh(&mut state, &index, &["k5", "k1", "gone", "k12"]);
    assert_eq!(state.filtered, [12]);
}

#[test]
fn all_includes_every_group() {
    let items = catalog(50);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    state.query = "item 0000".into();
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered.len(), 50);
    let groups: std::collections::BTreeSet<_> = state
        .filtered
        .iter()
        .map(|&item| items[item].group.as_str())
        .collect();
    assert_eq!(groups.len(), GROUPS);
}

#[test]
fn empty_results_are_safe() {
    let items = catalog(30);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    state.query = "zzz".into();
    refresh(&mut state, &index, &[]);
    assert!(state.filtered.is_empty());
    assert_eq!(state.selected, 0);
    assert_eq!(state.selected_item(), None);
    state.select_next(&index);
    state.select_prev(&index);
    assert_eq!(state.selected, 0);

    let empty = CatalogIndex::default();
    let mut blank = BrowserState::new(CategorySel::All);
    assert!(blank.refresh(&empty, 0, 0, Vec::new));
    assert!(blank.filtered.is_empty());
}

#[test]
fn clearing_the_query_restores_the_list_and_selection() {
    let items = catalog(3000);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    refresh(&mut state, &index, &[]);
    state.select_position(1007, &index);
    state.query = "needlecase".into();
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered, [7, 1007, 2007]);
    assert_eq!(state.selected, 1, "selected item survives the narrowing");

    state.select_position(2, &index);
    state.query = "item 00005".into();
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered.len(), 10);
    state.query.clear();
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered.len(), 3000);
    assert_eq!(
        state.selected, 2007,
        "the item selected before the query returns"
    );
}

#[test]
fn selection_is_restored_by_key_after_a_catalog_change() {
    let items = catalog(100);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    refresh(&mut state, &index, &[]);
    state.select_position(40, &index);

    let mut changed = catalog(100);
    changed.insert(
        0,
        Item {
            key: "new".into(),
            name: "New".into(),
            group: "g0".into(),
        },
    );
    changed.remove(10);
    let index = index_of(&changed);
    assert!(state.refresh(&index, 2, 1, Vec::new));
    assert_eq!(index.key(state.filtered[state.selected]), Some("k40"));
    assert_eq!(state.selected, 40, "one inserted, one removed before it");
}

#[test]
fn missing_key_selects_the_nearest_index() {
    let items = catalog(10);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    refresh(&mut state, &index, &[]);
    state.select_position(9, &index);

    let mut shrunk = catalog(10);
    shrunk.truncate(6);
    let index = index_of(&shrunk);
    state.refresh(&index, 2, 1, Vec::new);
    assert_eq!(state.selected, 5, "clamped to the last surviving item");

    state.select_position(2, &index);
    let mut without = catalog(6);
    without.remove(2);
    let index = index_of(&without);
    state.refresh(&index, 3, 1, Vec::new);
    assert_eq!(state.selected, 2, "same position when the key is gone");
}

#[test]
fn moving_wraps_and_tracks_the_key() {
    let items = catalog(3);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    refresh(&mut state, &index, &[]);
    state.select_prev(&index);
    assert_eq!(state.selected, 2);
    assert_eq!(state.selected_key.as_deref(), Some("k2"));
    state.select_next(&index);
    assert_eq!(state.selected, 0);
    assert!(state.select_key("k1", &index));
    assert_eq!(state.selected, 1);
    assert!(!state.select_key("nope", &index));
    state.select_position(99, &index);
    assert_eq!(state.selected, 1);
}

#[test]
fn category_change_resets_selection_and_default_focus_is_items() {
    let items = catalog(30);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    assert_eq!(state.focus, BrowserFocus::Items);
    refresh(&mut state, &index, &[]);
    state.select_position(12, &index);
    state.set_category(CategorySel::Group("g2".into()));
    refresh(&mut state, &index, &[]);
    assert_eq!(state.selected, 0);
    assert_eq!(state.selected_key.as_deref(), Some("k2"));
}

fn named_catalog(names: &[&str]) -> Vec<Item> {
    names
        .iter()
        .enumerate()
        .map(|(n, name)| Item {
            key: format!("k{n}"),
            name: (*name).to_owned(),
            group: format!("g{}", n % 2),
        })
        .collect()
}

#[test]
fn name_sort_is_case_insensitive_and_ties_keep_provider_order() {
    let items = named_catalog(&["banana", "Apple", "cherry", "apple", "Banana"]);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    state.sort = SortMode::Name;
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered, [1, 3, 0, 4, 2]);
    state.sort = SortMode::Provider;
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered, [0, 1, 2, 3, 4]);
}

#[test]
fn toggling_sort_keeps_the_selected_item() {
    let items = named_catalog(&["delta", "alpha", "charlie", "bravo"]);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    refresh(&mut state, &index, &[]);
    state.select_position(2, &index);
    assert_eq!(state.selected_key.as_deref(), Some("k2"));
    state.sort = SortMode::Name;
    assert!(refresh(&mut state, &index, &[]));
    assert_eq!(state.filtered, [1, 3, 2, 0]);
    assert_eq!(state.selected, 2);
    assert_eq!(state.selected_key.as_deref(), Some("k2"));
    state.select_position(0, &index);
    state.sort = SortMode::Provider;
    refresh(&mut state, &index, &[]);
    assert_eq!(index.key(state.filtered[state.selected]), Some("k1"));
}

#[test]
fn name_sort_applies_to_groups_and_search_results_but_not_favorites() {
    let items = named_catalog(&["delta x", "alpha y", "charlie x", "bravo x"]);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::Group("g0".into()));
    state.sort = SortMode::Name;
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered, [2, 0]);
    state.set_category(CategorySel::All);
    state.query = " X".into();
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered, [3, 2, 0]);
    state.query.clear();
    state.set_category(CategorySel::Favorites);
    refresh(&mut state, &index, &["k0", "k3", "k1"]);
    assert_eq!(state.filtered, [0, 3, 1], "favorites keep their order");
}

#[test]
fn name_sort_of_a_large_catalog_is_correct() {
    let items = catalog(54_000);
    let index = index_of(&items);
    let mut state = BrowserState::new(CategorySel::All);
    state.sort = SortMode::Name;
    state.query = "needlecase".into();
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered.len(), 54);
    assert!(state.filtered.windows(2).all(|pair| pair[0] < pair[1]));
    let mut expected: Vec<usize> = (0..54_000).collect();
    expected.sort_by_key(|&n| (items[n].name.to_lowercase(), n));
    state.query.clear();
    refresh(&mut state, &index, &[]);
    assert_eq!(state.filtered, expected);
}

#[test]
fn sort_mode_labels_toggle() {
    assert_eq!(SortMode::default(), SortMode::Provider);
    assert_eq!(SortMode::Provider.toggled(), SortMode::Name);
    assert_eq!(SortMode::Name.toggled().label(), "Sort: Provider");
}
