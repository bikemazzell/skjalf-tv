//! Shared browser behavior for Live, Movies, and Series: categories, search,
//! sorting, selection, and focus. Item data stays in each catalog.

use std::collections::HashSet;

use skjalf_core::browse::{BrowserFocus, BrowserState, CatalogIndex, CategorySel, SortMode};
use skjalf_core::catalog::{format_rating, group_name_order, ContentKey, ContentKind};
use skjalf_core::display::DisplayNames;
use skjalf_core::model::{Group, Playlist};
use skjalf_core::nav::View;
use skjalf_core::progress::Marker;

use super::{App, BrowserId, TextTarget, FAVORITES_GROUP};
use crate::input::Action;

/// Number of fixed rows (Favorites, All) before the provider groups.
const FIXED_CATEGORIES: usize = 2;

pub const UNAVAILABLE_ON_M3U: &str =
    "Movies and series need an Xtream account. This source provides live channels only.";

/// The sidebar's provider groups after the name filter, sort, and favorites,
/// cached by (catalog generation, filter text, sort mode, favorites generation).
#[derive(Debug, Default)]
pub struct GroupList {
    key: Option<(u64, String, SortMode, u64)>,
    /// Provider group positions in display order: favorite groups first.
    positions: Vec<usize>,
    /// How many leading entries of `positions` are favorite groups.
    favorite_count: usize,
    /// How many times the list was rebuilt (tests assert it stays put).
    pub builds: usize,
}

/// One list row: text and whether the item is a favorite.
pub struct ItemRow {
    pub label: String,
    pub favorite: bool,
    pub marker: Marker,
}

/// Row suffix for a real rating; zero and empty ratings add nothing.
fn real_rating(rating: Option<&str>) -> String {
    format_rating(rating)
        .map(|rating| format!("   {rating}"))
        .unwrap_or_default()
}

impl BrowserId {
    pub const ALL: [BrowserId; 3] = [BrowserId::Live, BrowserId::Movies, BrowserId::Series];

    pub fn kind(self) -> ContentKind {
        match self {
            BrowserId::Live => ContentKind::Live,
            BrowserId::Movies => ContentKind::Movie,
            BrowserId::Series => ContentKind::Series,
        }
    }

    pub fn view(self) -> View {
        match self {
            BrowserId::Live => View::Live,
            BrowserId::Movies => View::Movies,
            BrowserId::Series => View::Series,
        }
    }

    pub fn from_view(view: View) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.view() == view)
    }

    pub fn title(self) -> &'static str {
        match self {
            BrowserId::Live => "Live TV",
            BrowserId::Movies => "Movies",
            BrowserId::Series => "Series",
        }
    }
}

impl App {
    pub fn state(&self, id: BrowserId) -> &BrowserState {
        match id {
            BrowserId::Live => &self.live,
            BrowserId::Movies => &self.movies,
            BrowserId::Series => &self.series,
        }
    }

    pub fn state_mut(&mut self, id: BrowserId) -> &mut BrowserState {
        match id {
            BrowserId::Live => &mut self.live,
            BrowserId::Movies => &mut self.movies,
            BrowserId::Series => &mut self.series,
        }
    }

    /// Install a playlist, rebuild the search index, and keep the selection.
    pub fn set_playlist(&mut self, playlist: Playlist) {
        self.live_index = CatalogIndex::build(playlist.channels.iter().map(|channel| {
            (
                channel.id.as_str(),
                channel.name.as_str(),
                channel.group_id.as_str(),
            )
        }));
        self.live_group_order = group_name_order(&playlist.groups);
        self.live_group_display =
            DisplayNames::build(playlist.groups.iter().map(|group| group.name.as_str()));
        self.playlist = Some(playlist);
        self.catalog_generation += 1;
        if let CategorySel::Group(id) = &self.live.category {
            let known = self
                .playlist
                .as_ref()
                .is_some_and(|pl| pl.groups.iter().any(|group| group.id == *id));
            if !known {
                self.live.set_category(CategorySel::All);
            }
        }
        self.live_refresh();
    }

    /// Recompute the visible items if the catalog, category, query, sort, or
    /// favorites changed since the last call. Cheap when nothing changed.
    pub fn refresh_browser(&mut self, id: BrowserId) {
        let kind = id.kind();
        let favorites = &self.favorites;
        let source_id = self
            .settings
            .source
            .as_ref()
            .map(|source| source.id.as_str());
        let favorites_generation = self.favorites_generation;
        let (state, index, generation) = match id {
            BrowserId::Live => (&mut self.live, &self.live_index, self.catalog_generation),
            BrowserId::Movies => (
                &mut self.movies,
                self.movie_slot.index(),
                self.movie_slot.installed,
            ),
            BrowserId::Series => (
                &mut self.series,
                self.series_slot.index(),
                self.series_slot.installed,
            ),
        };
        state.refresh(index, generation, favorites_generation, || {
            favorites
                .iter()
                .filter(|(source, _)| Some(source.as_str()) == source_id)
                .filter_map(|(_, key)| ContentKey::id_of_favorite(kind, key).map(str::to_owned))
                .collect()
        });
        self.refresh_group_list(id);
    }

    fn refresh_group_list(&mut self, id: BrowserId) {
        let generation = match id {
            BrowserId::Live => self.catalog_generation,
            BrowserId::Movies => self.movie_slot.installed,
            BrowserId::Series => self.series_slot.installed,
        };
        let state = self.state(id);
        let key = (
            generation,
            state.group_filter.trim().to_lowercase(),
            state.sort,
            self.favorites_generation,
        );
        let slot = id as usize;
        if self.group_lists[slot].key.as_ref() == Some(&key) {
            return;
        }
        let groups = self.browser_groups(id);
        let order = self.group_order(id);
        let sorted = key.2 == SortMode::Name && order.len() == groups.len();
        let favorite_ids = self.favorite_group_ids(id);
        let (mut positions, others): (Vec<usize>, Vec<usize>) = (0..groups.len())
            .map(|position| if sorted { order[position] } else { position })
            .filter(|&index| key.1.is_empty() || groups[index].name.to_lowercase().contains(&key.1))
            .partition(|&index| favorite_ids.contains(groups[index].id.as_str()));
        let favorite_count = positions.len();
        positions.extend(others);
        let list = &mut self.group_lists[slot];
        list.key = Some(key);
        list.positions = positions;
        list.favorite_count = favorite_count;
        list.builds += 1;
    }

    fn favorite_group_ids(&self, id: BrowserId) -> HashSet<&str> {
        let Some(source_id) = self.source_id() else {
            return HashSet::new();
        };
        self.favorites
            .iter()
            .filter(|(source, _)| source == source_id)
            .filter_map(|(_, key)| ContentKey::group_id_of_favorite(id.kind(), key))
            .collect()
    }

    pub fn group_list_builds(&self, id: BrowserId) -> usize {
        self.group_lists[id as usize].builds
    }

    pub fn live_refresh(&mut self) {
        self.refresh_browser(BrowserId::Live);
    }

    pub fn refresh_browsers(&mut self) {
        for id in BrowserId::ALL {
            self.refresh_browser(id);
        }
    }

    pub fn browser_groups(&self, id: BrowserId) -> &[Group] {
        match id {
            BrowserId::Live => self
                .playlist
                .as_ref()
                .map_or(&[], |playlist| playlist.groups.as_slice()),
            BrowserId::Movies => self.movie_groups(),
            BrowserId::Series => self.series_groups(),
        }
    }

    /// Message that replaces the browser when it has nothing to show.
    pub fn browser_notice(&self, id: BrowserId) -> Option<String> {
        match id {
            BrowserId::Live => None,
            BrowserId::Movies => self.catalog_notice(
                "movies",
                self.movie_slot.loaded.is_some(),
                &self.movie_slot.status,
            ),
            BrowserId::Series => self.catalog_notice(
                "series",
                self.series_slot.loaded.is_some(),
                &self.series_slot.status,
            ),
        }
    }

    fn catalog_notice(
        &self,
        noun: &str,
        loaded: bool,
        status: &super::LoadStatus,
    ) -> Option<String> {
        if !self.has_xtream_source() {
            return Some(UNAVAILABLE_ON_M3U.into());
        }
        if loaded {
            return None;
        }
        Some(match status {
            super::LoadStatus::Failed(message) => {
                format!("Could not load {noun}: {message} Select to retry.")
            }
            _ => format!("Loading {noun}..."),
        })
    }

    /// Catalog position count and label data for a visible row.
    pub fn browser_row(&self, id: BrowserId, item: usize) -> Option<ItemRow> {
        let source_id = self.source_id();
        let favorite = |key: &ContentKey| {
            source_id.is_some_and(|source| self.favorites.contains(source, &key.favorite_key()))
        };
        match id {
            BrowserId::Live => {
                let channel = self.playlist.as_ref()?.channels.get(item)?;
                let num = channel
                    .num
                    .map(|n| format!("{n:>4}  "))
                    .unwrap_or_else(|| "     ".into());
                Some(ItemRow {
                    label: format!(
                        "{num}{}",
                        self.live_index
                            .display_name(item, &channel.name)
                            .replace(['\n', '\r'], " ")
                    ),
                    favorite: favorite(&ContentKey::new(ContentKind::Live, &channel.id)),
                    marker: Marker::None,
                })
            }
            BrowserId::Movies => {
                let movie = self.movie_slot.loaded.as_ref()?.data.movies.get(item)?;
                let rating = real_rating(movie.rating.as_deref());
                Some(ItemRow {
                    label: format!(
                        "{}{rating}",
                        self.movie_slot
                            .index()
                            .display_name(item, &movie.name)
                            .replace(['\n', '\r'], " ")
                    ),
                    favorite: favorite(&ContentKey::new(ContentKind::Movie, &movie.id)),
                    marker: self.markers.movie(&movie.id),
                })
            }
            BrowserId::Series => {
                let series = self.series_slot.loaded.as_ref()?.data.series.get(item)?;
                let rating = real_rating(series.rating.as_deref());
                Some(ItemRow {
                    label: format!(
                        "{}{rating}",
                        self.series_slot
                            .index()
                            .display_name(item, &series.name)
                            .replace(['\n', '\r'], " ")
                    ),
                    favorite: favorite(&ContentKey::new(ContentKind::Series, &series.id)),
                    marker: self.markers.series(&series.id),
                })
            }
        }
    }

    /// Key and display name of the highlighted item.
    pub fn highlighted_item(&self, id: BrowserId) -> Option<(ContentKey, String)> {
        let item = self.state(id).selected_item()?;
        match id {
            BrowserId::Live => {
                let channel = self.playlist.as_ref()?.channels.get(item)?;
                Some((
                    ContentKey::new(ContentKind::Live, &channel.id),
                    channel.name.clone(),
                ))
            }
            BrowserId::Movies => {
                let movie = self.movie_slot.loaded.as_ref()?.data.movies.get(item)?;
                Some((
                    ContentKey::new(ContentKind::Movie, &movie.id),
                    movie.name.clone(),
                ))
            }
            BrowserId::Series => {
                let item = self.series_slot.loaded.as_ref()?.data.series.get(item)?;
                Some((
                    ContentKey::new(ContentKind::Series, &item.id),
                    item.name.clone(),
                ))
            }
        }
    }

    /// Channel ids visible in the current Live category and query.
    pub fn group_channel_ids(&self) -> Vec<String> {
        let Some(pl) = &self.playlist else {
            return Vec::new();
        };
        self.live
            .filtered
            .iter()
            .map(|&index| pl.channels[index].id.clone())
            .collect()
    }

    /// Playlist positions visible in the current Live category and query.
    pub fn group_channel_indices(&self) -> &[usize] {
        &self.live.filtered
    }

    /// Sidebar rows: Favorites, All, then the provider groups.
    pub fn categories_len(&self, id: BrowserId) -> usize {
        self.group_lists[id as usize].positions.len() + FIXED_CATEGORIES
    }

    /// Group positions in name order (computed once per catalog load).
    fn group_order(&self, id: BrowserId) -> &[usize] {
        match id {
            BrowserId::Live => &self.live_group_order,
            BrowserId::Movies => self
                .movie_slot
                .loaded
                .as_ref()
                .map_or(&[], |loaded| loaded.group_order.as_slice()),
            BrowserId::Series => self
                .series_slot
                .loaded
                .as_ref()
                .map_or(&[], |loaded| loaded.group_order.as_slice()),
        }
    }

    /// The group shown at `position` in the filtered, sorted sidebar.
    fn displayed_group(&self, id: BrowserId, position: usize) -> Option<&Group> {
        let index = *self.group_lists[id as usize].positions.get(position)?;
        self.browser_groups(id).get(index)
    }

    /// A group's name as drawn.
    fn group_display_name<'a>(&'a self, id: BrowserId, index: usize, logical: &'a str) -> &'a str {
        let display = match id {
            BrowserId::Live => &self.live_group_display,
            BrowserId::Movies => match &self.movie_slot.loaded {
                Some(loaded) => &loaded.group_display,
                None => return logical,
            },
            BrowserId::Series => match &self.series_slot.loaded {
                Some(loaded) => &loaded.group_display,
                None => return logical,
            },
        };
        display.get(index, logical)
    }

    pub fn category_at(&self, id: BrowserId, row: usize) -> Option<CategorySel> {
        match row {
            0 => Some(CategorySel::Favorites),
            1 => Some(CategorySel::All),
            _ => self
                .displayed_group(id, row - FIXED_CATEGORIES)
                .map(|group| CategorySel::Group(group.id.clone())),
        }
    }

    /// True when the sidebar row is a favorite provider group.
    pub fn category_favorite(&self, id: BrowserId, row: usize) -> bool {
        row >= FIXED_CATEGORIES
            && row - FIXED_CATEGORIES < self.group_lists[id as usize].favorite_count
    }

    pub fn category_label(&self, id: BrowserId, row: usize) -> Option<String> {
        match row {
            0 => Some(FAVORITES_GROUP.into()),
            1 => Some("All".into()),
            _ => {
                let index = *self.group_lists[id as usize]
                    .positions
                    .get(row - FIXED_CATEGORIES)?;
                let group = self.browser_groups(id).get(index)?;
                Some(self.group_display_name(id, index, &group.name).to_owned())
            }
        }
    }

    pub fn category_row(&self, id: BrowserId) -> usize {
        match &self.state(id).category {
            CategorySel::Favorites => 0,
            CategorySel::All => 1,
            CategorySel::Group(group_id) => {
                let groups = self.browser_groups(id);
                self.group_lists[id as usize]
                    .positions
                    .iter()
                    .position(|&index| groups.get(index).is_some_and(|group| group.id == *group_id))
                    .map_or(self.categories_len(id), |position| {
                        position + FIXED_CATEGORIES
                    })
            }
        }
    }

    /// Heading for the browser: the selected category's own name.
    pub fn category_title(&self, id: BrowserId) -> String {
        match &self.state(id).category {
            CategorySel::Favorites => FAVORITES_GROUP.into(),
            CategorySel::All => "All".into(),
            CategorySel::Group(group_id) => self
                .browser_groups(id)
                .iter()
                .enumerate()
                .find(|(_, group)| group.id == *group_id)
                .map_or_else(String::new, |(index, group)| {
                    self.group_display_name(id, index, &group.name).to_owned()
                }),
        }
    }

    /// True when a filter is typed and no provider group matches it.
    pub fn no_groups_match(&self, id: BrowserId) -> bool {
        !self.state(id).group_filter.trim().is_empty()
            && self.group_lists[id as usize].positions.is_empty()
    }

    pub fn set_group_filter(&mut self, id: BrowserId, filter: String) {
        self.state_mut(id).group_filter = filter;
        self.refresh_browser(id);
    }

    /// Down from the filter field: the first match when the filter has
    /// matches, else the selected category (the first visible entry when
    /// the filter hides it).
    fn leave_group_filter_down(&mut self, id: BrowserId) {
        let filtering = !self.state(id).group_filter.trim().is_empty();
        if filtering && !self.no_groups_match(id) {
            return self.focus_first_matching_group(id);
        }
        self.state_mut(id).focus = BrowserFocus::Categories;
        if self.category_row(id) >= self.categories_len(id) {
            if let Some(first) = self.category_at(id, 0) {
                self.set_category(id, first);
            }
        }
    }

    /// Select the first group that matches the filter and focus the sidebar.
    pub(super) fn focus_first_matching_group(&mut self, id: BrowserId) {
        let first = (!self.state(id).group_filter.trim().is_empty())
            .then(|| self.category_at(id, FIXED_CATEGORIES))
            .flatten();
        match first {
            Some(category) => {
                self.set_category(id, category);
                self.state_mut(id).focus = BrowserFocus::Categories;
            }
            None if self.state(id).group_filter.trim().is_empty() => {
                self.state_mut(id).focus = BrowserFocus::Categories;
            }
            None => self.state_mut(id).focus = BrowserFocus::GroupFilter,
        }
    }

    pub fn set_category(&mut self, id: BrowserId, category: CategorySel) {
        self.state_mut(id).set_category(category);
        self.refresh_browser(id);
    }

    pub fn set_query(&mut self, id: BrowserId, query: String) {
        self.state_mut(id).query = query;
        self.refresh_browser(id);
    }

    /// Switch between provider order and name order and remember the choice.
    pub fn toggle_sort(&mut self, id: BrowserId) {
        let sort: SortMode = self.state(id).sort.toggled();
        self.state_mut(id).sort = sort;
        match id {
            BrowserId::Live => self.settings.sort_modes.live = sort,
            BrowserId::Movies => self.settings.sort_modes.movies = sort,
            BrowserId::Series => self.settings.sort_modes.series = sort,
        }
        self.save_settings();
        self.refresh_browser(id);
    }

    /// Land on Favorites when this source has any, else the first non-empty
    /// provider group (favorites are per-source).
    pub(super) fn select_startup_group(&mut self) {
        let has_favs = self.source_id().is_some_and(|sid| {
            self.favorites.iter().any(|(s, key)| {
                s == sid && ContentKey::id_of_favorite(ContentKind::Live, key).is_some()
            })
        });
        let category = if has_favs {
            CategorySel::Favorites
        } else {
            self.first_nonempty_group().unwrap_or(CategorySel::All)
        };
        self.set_category(BrowserId::Live, category);
    }

    fn first_nonempty_group(&self) -> Option<CategorySel> {
        let pl = self.playlist.as_ref()?;
        pl.groups
            .iter()
            .find(|group| {
                pl.channels
                    .iter()
                    .any(|channel| channel.group_id == group.id)
            })
            .map(|group| CategorySel::Group(group.id.clone()))
    }

    pub(super) fn reset_live(&mut self) {
        self.live = BrowserState::default();
        self.live.sort = self.settings.sort_modes.live;
        self.select_startup_group();
    }

    pub(super) fn enter_browser(&mut self, id: BrowserId) {
        self.state_mut(id).focus = BrowserFocus::Items;
        self.router.push(id.view());
        match id {
            BrowserId::Live => {}
            BrowserId::Movies => self.record_opened(super::RefreshKind::Movies),
            BrowserId::Series => self.record_opened(super::RefreshKind::Series),
        }
        self.retry_catalog(id);
    }

    pub(super) fn handle_browser(&mut self, id: BrowserId, action: Action, now_ms: u64) {
        if self.browser_notice(id).is_some() {
            match action {
                Action::Select => self.retry_catalog(id),
                Action::Back => {
                    self.router.back();
                }
                _ => {}
            }
            return;
        }
        match action {
            Action::Up | Action::Down => self.browser_step(id, action, false),
            Action::Left | Action::Right => self.browser_horizontal(id, action),
            Action::Select => self.browser_select(id, now_ms),
            Action::Epg => self.start_search_edit(id),
            Action::ChUp => self.browser_page(id, false),
            Action::ChDown => self.browser_page(id, true),
            Action::Digit(d) if id == BrowserId::Live => {
                // Buffer like the player; Select or timeout commits.
                self.zap.push(d, now_ms);
                self.show_zap_osd(now_ms);
            }
            Action::Back => {
                if self.state(id).focus != BrowserFocus::Items {
                    self.state_mut(id).focus = BrowserFocus::Items;
                } else {
                    self.router.back();
                }
            }
            Action::Favorite if self.state(id).focus == BrowserFocus::Categories => {
                self.toggle_group_favorite(id, now_ms);
            }
            Action::Favorite => {
                // Target the highlighted item, not the last played one.
                if let Some((key, name)) = self.highlighted_item(id) {
                    self.toggle_favorite(&key, &name, now_ms);
                }
            }
            _ => {}
        }
        self.refresh_browser(id);
    }

    /// The one-line control hint for the focused part of a browser.
    pub fn browser_hint(&self, id: BrowserId) -> String {
        if self.editing.is_some() {
            return "Type, then Enter or B to finish".into();
        }
        match self.state(id).focus {
            BrowserFocus::Items => {
                let favorite = self.highlighted_item(id).is_some_and(|(key, _)| {
                    self.source_id()
                        .is_some_and(|source| self.favorites.contains(source, &key.favorite_key()))
                });
                let verb = if favorite { "Unfavorite" } else { "Favorite" };
                format!("A Open   Y {verb}   X Search   L1/R1 Page   B Back")
            }
            BrowserFocus::Categories => {
                let row = self.category_row(id);
                if row < FIXED_CATEGORIES || row >= self.categories_len(id) {
                    return "A Open   X Search   B Back".into();
                }
                let verb = if self.category_favorite(id, row) {
                    "Unfavorite group"
                } else {
                    "Favorite group"
                };
                format!("A Open   Y {verb}   X Search   B Back")
            }
            BrowserFocus::GroupFilter | BrowserFocus::Search => "A Edit   B Back".into(),
            BrowserFocus::GroupFilterClear | BrowserFocus::Clear | BrowserFocus::Sort => {
                "A Select   B Back".into()
            }
        }
    }

    /// Toggle the selected provider group; Favorites and All ignore it.
    fn toggle_group_favorite(&mut self, id: BrowserId, now_ms: u64) {
        let CategorySel::Group(group_id) = &self.state(id).category else {
            return;
        };
        let stored = ContentKey::group_favorite_key(id.kind(), group_id);
        let name = self.category_title(id);
        self.toggle_favorite_stored(&stored, &name, now_ms);
    }

    /// A held-direction repeat. Only Up/Down inside the category list or the
    /// item list repeat, and a repeat never leaves the list.
    pub fn handle_repeat(&mut self, action: Action) {
        if self.router.current() == View::SeriesDetails {
            if matches!(action, Action::Up | Action::Down) {
                self.series_details_step(action, true);
            }
            return;
        }
        let Some(id) = BrowserId::from_view(self.router.current()) else {
            return;
        };
        let repeats_here = self.editing.is_none()
            && self.browser_notice(id).is_none()
            && matches!(action, Action::Up | Action::Down)
            && matches!(
                self.state(id).focus,
                BrowserFocus::Categories | BrowserFocus::Items
            );
        if repeats_here {
            self.browser_step(id, action, true);
            self.refresh_browser(id);
        }
    }

    fn browser_step(&mut self, id: BrowserId, action: Action, repeated: bool) {
        let forward = action == Action::Down;
        match self.state(id).focus {
            BrowserFocus::Categories => {
                let rows = self.categories_len(id);
                let row = self.category_row(id);
                if !forward && row == 0 {
                    if !repeated {
                        self.state_mut(id).focus = BrowserFocus::GroupFilter;
                    }
                    return;
                }
                let next = match (row < rows, forward) {
                    (true, true) => (row + 1) % rows,
                    (true, false) => row - 1,
                    (false, true) => FIXED_CATEGORIES.min(rows - 1),
                    (false, false) => rows - 1,
                };
                if let Some(category) = self.category_at(id, next) {
                    self.set_category(id, category);
                }
            }
            BrowserFocus::GroupFilter | BrowserFocus::GroupFilterClear => {
                if forward {
                    self.leave_group_filter_down(id);
                }
            }
            BrowserFocus::Items => {
                let (state, index) = self.state_and_index(id);
                if forward {
                    state.select_next(index);
                } else if state.selected > 0 {
                    state.select_prev(index);
                } else if !repeated {
                    state.focus = BrowserFocus::Search;
                }
            }
            BrowserFocus::Search | BrowserFocus::Clear | BrowserFocus::Sort => {
                if forward {
                    self.state_mut(id).focus = BrowserFocus::Items;
                }
            }
        }
    }

    fn state_and_index(&mut self, id: BrowserId) -> (&mut BrowserState, &CatalogIndex) {
        match id {
            BrowserId::Live => (&mut self.live, &self.live_index),
            BrowserId::Movies => (&mut self.movies, self.movie_slot.index()),
            BrowserId::Series => (&mut self.series, self.series_slot.index()),
        }
    }

    fn browser_horizontal(&mut self, id: BrowserId, action: Action) {
        let right = action == Action::Right;
        let state = self.state_mut(id);
        state.focus = match (state.focus, right) {
            (BrowserFocus::Categories, true) if state.filtered.is_empty() => BrowserFocus::Search,
            (BrowserFocus::Categories, true) => BrowserFocus::Items,
            (BrowserFocus::Categories, false) => BrowserFocus::GroupFilter,
            (BrowserFocus::Items, false) | (BrowserFocus::Search, false) => {
                BrowserFocus::Categories
            }
            (BrowserFocus::GroupFilter, true) => BrowserFocus::GroupFilterClear,
            (BrowserFocus::GroupFilterClear, true) => BrowserFocus::Search,
            (BrowserFocus::GroupFilterClear, false) => BrowserFocus::GroupFilter,
            (BrowserFocus::Search, true) => BrowserFocus::Clear,
            (BrowserFocus::Clear, true) => BrowserFocus::Sort,
            (BrowserFocus::Clear, false) => BrowserFocus::Search,
            (BrowserFocus::Sort, false) => BrowserFocus::Clear,
            (focus, _) => focus,
        };
    }

    fn browser_select(&mut self, id: BrowserId, now_ms: u64) {
        match self.state(id).focus {
            BrowserFocus::Categories => self.browser_horizontal(id, Action::Right),
            BrowserFocus::GroupFilter => self.start_group_filter_edit(id),
            BrowserFocus::GroupFilterClear => {
                self.set_group_filter(id, String::new());
                self.state_mut(id).focus = BrowserFocus::GroupFilter;
            }
            BrowserFocus::Search => self.start_search_edit(id),
            BrowserFocus::Clear => self.set_query(id, String::new()),
            BrowserFocus::Sort => self.toggle_sort(id),
            BrowserFocus::Items => match id {
                BrowserId::Live => self.live_select_item(now_ms),
                BrowserId::Movies => {
                    if let Some(item) = self.movies.selected_item() {
                        self.open_movie_details(item);
                    }
                }
                BrowserId::Series => {
                    if let Some(item) = self.series.selected_item() {
                        self.open_series_details(item);
                    }
                }
            },
        }
    }

    /// Touch or mouse activation of a visible row.
    pub fn click_item(&mut self, id: BrowserId, position: usize, now_ms: u64) {
        self.end_text_edit();
        let (state, index) = self.state_and_index(id);
        state.select_position(position, index);
        state.focus = BrowserFocus::Items;
        self.browser_select(id, now_ms);
    }

    fn live_select_item(&mut self, now_ms: u64) {
        // Commit any buffered zap digits, else play the highlight.
        if let Some(num) = self.zap.force() {
            self.zap_to(num, now_ms);
        } else if let Some(id) = self.group_channel_ids().get(self.live.selected).cloned() {
            if let Some(ch) = self.playlist.as_ref().and_then(|p| p.find(&id)) {
                let ch = ch.clone();
                self.play(&ch, now_ms);
            }
        }
    }

    /// Start typing in a browser's search field (Steam keyboard in Game Mode).
    pub fn start_search_edit(&mut self, id: BrowserId) {
        self.state_mut(id).focus = BrowserFocus::Search;
        self.start_text_edit(TextTarget::Search(id));
    }

    /// Start typing in the sidebar's group filter.
    pub fn start_group_filter_edit(&mut self, id: BrowserId) {
        self.state_mut(id).focus = BrowserFocus::GroupFilter;
        self.start_text_edit(TextTarget::GroupFilter(id));
    }

    pub(super) fn start_text_edit(&mut self, target: TextTarget) {
        if self.editing != Some(target) {
            self.editing = Some(target);
            self.focus_request = Some(target);
            self.zap = Default::default();
            self.open_steam_keyboard();
        }
    }

    /// True once after an edit starts on `target`; the view then focuses it.
    pub fn take_focus_request(&mut self, target: TextTarget) -> bool {
        if self.focus_request == Some(target) {
            self.focus_request = None;
            true
        } else {
            false
        }
    }

    /// One visible page up or down in the sidebar or item list; no wrap.
    fn browser_page(&mut self, id: BrowserId, forward: bool) {
        match self.state(id).focus {
            BrowserFocus::Categories => {
                let page = self.page_rows[id as usize][0].max(1);
                let rows = self.categories_len(id);
                let row = self.category_row(id).min(rows - 1);
                let target = if forward {
                    (row + page).min(rows - 1)
                } else {
                    row.saturating_sub(page)
                };
                if let Some(category) = self.category_at(id, target) {
                    self.set_category(id, category);
                }
            }
            BrowserFocus::Items => {
                let page = self.page_rows[id as usize][1].max(1);
                let (state, index) = self.state_and_index(id);
                let len = state.filtered.len();
                if len > 0 {
                    let target = if forward {
                        (state.selected + page).min(len - 1)
                    } else {
                        state.selected.saturating_sub(page)
                    };
                    state.select_position(target, index);
                }
            }
            _ => {}
        }
    }

    /// Keep the Live selection aligned with what is playing. If the channel
    /// is hidden by the query or category, show it: drop the query first,
    /// then move to the channel's own group.
    pub(super) fn sync_live_to_channel(&mut self, channel_id: &str) {
        self.live_refresh();
        if self.live.select_key(channel_id, &self.live_index) {
            return;
        }
        if !self.live.query.is_empty() {
            self.live.query.clear();
            self.live_refresh();
            if self.live.select_key(channel_id, &self.live_index) {
                return;
            }
        }
        let group = self
            .playlist
            .as_ref()
            .and_then(|pl| pl.find(channel_id))
            .map(|channel| CategorySel::Group(channel.group_id.clone()));
        if let Some(group) = group {
            self.set_category(BrowserId::Live, group);
            self.live.select_key(channel_id, &self.live_index);
        }
    }
}
