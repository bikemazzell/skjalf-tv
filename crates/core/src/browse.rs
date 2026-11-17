//! Pure browser state shared by the Live, Movies, and Series screens.
//!
//! The state owns the category, the query, the selection, and the cached list
//! of visible item indices. Filtering runs only when [`BrowserState::refresh`]
//! sees a changed cache key, never while painting.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::display::DisplayNames;

/// Which slice of the catalog a browser shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CategorySel {
    Favorites,
    #[default]
    All,
    /// Provider group id.
    Group(String),
}

/// How a browser orders items other than Favorites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    /// The provider's own order.
    #[default]
    Provider,
    /// Case-insensitive name, A to Z; ties keep provider order.
    Name,
}

impl SortMode {
    pub fn toggled(self) -> Self {
        match self {
            SortMode::Provider => SortMode::Name,
            SortMode::Name => SortMode::Provider,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Provider => "Sort: Provider",
            SortMode::Name => "Sort: A-Z",
        }
    }
}

/// Which part of a browser screen has controller focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserFocus {
    Categories,
    /// The "Filter groups" field above the sidebar.
    GroupFilter,
    /// The small clear control beside the group filter field.
    GroupFilterClear,
    Search,
    Clear,
    Sort,
    #[default]
    Items,
}

/// Per-catalog data a filter needs: lowercase names, group ids, and keys.
/// Build it once when a catalog loads.
#[derive(Debug, Default)]
pub struct CatalogIndex {
    names_lower: Vec<String>,
    group_ids: Vec<String>,
    keys: Vec<String>,
    display: DisplayNames,
    by_key: HashMap<String, usize>,
    /// Item positions sorted by lowercase name; ties keep provider order.
    name_order: Vec<usize>,
}

impl CatalogIndex {
    /// Build from `(stable key, display name, group id)` triples.
    pub fn build<'a>(items: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>) -> Self {
        let mut index = Self::default();
        for (position, (key, name, group_id)) in items.into_iter().enumerate() {
            index.names_lower.push(name.to_lowercase());
            index.display.push(name);
            index.group_ids.push(group_id.to_owned());
            index.keys.push(key.to_owned());
            index.by_key.entry(key.to_owned()).or_insert(position);
        }
        index.name_order = (0..index.names_lower.len()).collect();
        index
            .name_order
            .sort_by(|&a, &b| index.names_lower[a].cmp(&index.names_lower[b]));
        index
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The name as it should be drawn (right-to-left text in visual order).
    pub fn display_name<'a>(&'a self, item: usize, logical: &'a str) -> &'a str {
        self.display.get(item, logical)
    }

    pub fn key(&self, item: usize) -> Option<&str> {
        self.keys.get(item).map(String::as_str)
    }

    fn ordered(&self, sort: SortMode) -> Box<dyn Iterator<Item = usize> + '_> {
        match sort {
            SortMode::Provider => Box::new(0..self.len()),
            SortMode::Name => Box::new(self.name_order.iter().copied()),
        }
    }

    pub fn position(&self, key: &str) -> Option<usize> {
        self.by_key.get(key).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilterKey {
    catalog_generation: u64,
    category: CategorySel,
    needle: String,
    sort: SortMode,
    favorites_generation: u64,
}

#[derive(Debug, Default)]
pub struct BrowserState {
    pub category: CategorySel,
    pub query: String,
    /// Case-insensitive substring filter for the sidebar's provider groups.
    pub group_filter: String,
    pub sort: SortMode,
    pub selected: usize,
    pub selected_key: Option<String>,
    pub focus: BrowserFocus,
    /// Catalog positions that match the category and query, in display order.
    pub filtered: Vec<usize>,
    cache_key: Option<FilterKey>,
}

impl BrowserState {
    pub fn new(category: CategorySel) -> Self {
        Self {
            category,
            ..Default::default()
        }
    }

    /// Pick a category; the selection returns to the first item.
    pub fn set_category(&mut self, category: CategorySel) {
        if self.category != category {
            self.category = category;
            self.selected = 0;
            self.selected_key = None;
        }
    }

    /// Catalog position of the selected item.
    pub fn selected_item(&self) -> Option<usize> {
        self.filtered.get(self.selected).copied()
    }

    /// Recompute `filtered` when the cache key changed; returns whether it ran.
    /// `favorite_keys` supplies the favorites in display order and is called
    /// only when the Favorites category needs a recompute.
    pub fn refresh(
        &mut self,
        index: &CatalogIndex,
        catalog_generation: u64,
        favorites_generation: u64,
        favorite_keys: impl FnOnce() -> Vec<String>,
    ) -> bool {
        let key = FilterKey {
            catalog_generation,
            category: self.category.clone(),
            needle: self.query.trim().to_lowercase(),
            sort: self.sort,
            favorites_generation,
        };
        if self.cache_key.as_ref() == Some(&key) {
            return false;
        }
        let matches =
            |item: usize| key.needle.is_empty() || index.names_lower[item].contains(&key.needle);
        self.filtered = match &key.category {
            CategorySel::Favorites => favorite_keys()
                .iter()
                .filter_map(|favorite| index.position(favorite))
                .filter(|&item| matches(item))
                .collect(),
            CategorySel::All => index
                .ordered(key.sort)
                .filter(|&item| matches(item))
                .collect(),
            CategorySel::Group(group_id) => index
                .ordered(key.sort)
                .filter(|&item| index.group_ids[item] == *group_id && matches(item))
                .collect(),
        };
        self.cache_key = Some(key);
        self.restore_selection(index);
        true
    }

    /// Keep the selected key selected. If it is gone, clamp to the nearest
    /// index and remember the key, so clearing a query returns to the item.
    fn restore_selection(&mut self, index: &CatalogIndex) {
        let found = self.selected_key.as_deref().and_then(|key| {
            let item = index.position(key)?;
            self.filtered
                .iter()
                .position(|&candidate| candidate == item)
        });
        match found {
            Some(position) => self.selected = position,
            None => {
                self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
                if self.selected_key.is_none() {
                    self.record_selected_key(index);
                }
            }
        }
    }

    fn record_selected_key(&mut self, index: &CatalogIndex) {
        self.selected_key = self
            .selected_item()
            .and_then(|item| index.key(item))
            .map(str::to_owned);
    }

    /// Select by list position; out-of-range positions are ignored.
    pub fn select_position(&mut self, position: usize, index: &CatalogIndex) {
        if position < self.filtered.len() {
            self.selected = position;
            self.record_selected_key(index);
        }
    }

    /// Select an item by key if it is in the current list.
    pub fn select_key(&mut self, key: &str, index: &CatalogIndex) -> bool {
        let position = index.position(key).and_then(|item| {
            self.filtered
                .iter()
                .position(|&candidate| candidate == item)
        });
        if let Some(position) = position {
            self.select_position(position, index);
        }
        position.is_some()
    }

    pub fn select_next(&mut self, index: &CatalogIndex) {
        if !self.filtered.is_empty() {
            self.select_position((self.selected + 1) % self.filtered.len(), index);
        }
    }

    pub fn select_prev(&mut self, index: &CatalogIndex) {
        if !self.filtered.is_empty() {
            let len = self.filtered.len();
            self.select_position((self.selected + len - 1) % len, index);
        }
    }
}
