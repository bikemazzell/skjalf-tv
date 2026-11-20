//! Main screen: the destination buttons and the placeholder destinations.

use skjalf_core::catalog::{format_rating, SeriesItem};
use skjalf_core::progress::{Marker, ProgressEntry, ProgressKind};

use super::{App, BrowserId};
use crate::input::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainButton {
    #[default]
    LiveTv,
    Movies,
    Series,
    Settings,
    Exit,
}

impl MainButton {
    pub const ALL: [MainButton; 5] = [
        MainButton::LiveTv,
        MainButton::Movies,
        MainButton::Series,
        MainButton::Settings,
        MainButton::Exit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MainButton::LiveTv => "Live TV",
            MainButton::Movies => "Movies",
            MainButton::Series => "Series",
            MainButton::Settings => "Settings",
            MainButton::Exit => "Exit",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|button| *button == self)
            .unwrap_or_default()
    }
}

/// A horizontal content row below the Main buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MainRow {
    Continue,
    LatestMovies,
    LatestSeries,
}

impl MainRow {
    const ALL: [MainRow; 3] = [
        MainRow::Continue,
        MainRow::LatestMovies,
        MainRow::LatestSeries,
    ];

    pub fn title(self) -> &'static str {
        match self {
            MainRow::Continue => "Continue Watching",
            MainRow::LatestMovies => "Latest Movies",
            MainRow::LatestSeries => "Latest Series",
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// "Rating 8" for a real rating; empty and zero ratings show nothing.
pub fn rating_detail(rating: Option<&str>) -> Option<String> {
    format_rating(rating).map(|rating| format!("Rating {rating}"))
}

/// One tile in a Main row.
pub struct RowTile {
    pub title: String,
    pub detail: Option<String>,
    pub marker: Marker,
    /// Art key: a poster URL or a captured frame.
    pub art: Option<String>,
}

impl App {
    fn main_row_len(&self, row: MainRow) -> usize {
        match row {
            MainRow::Continue => self.continue_items.len(),
            MainRow::LatestMovies => self
                .movie_slot
                .loaded
                .as_ref()
                .map_or(0, |l| l.latest.len()),
            MainRow::LatestSeries => self
                .series_slot
                .loaded
                .as_ref()
                .map_or(0, |l| l.latest.len()),
        }
    }

    /// Rows that have items, in display order.
    pub fn main_rows(&self) -> Vec<MainRow> {
        MainRow::ALL
            .into_iter()
            .filter(|row| self.main_row_len(*row) > 0)
            .collect()
    }

    /// Selected tile of a row, clamped to the row's current length.
    pub fn main_row_selection(&self, row: MainRow) -> usize {
        self.main_row_sel[row.slot()].min(self.main_row_len(row).saturating_sub(1))
    }

    pub fn main_row_tiles(&self, row: MainRow) -> Vec<RowTile> {
        match row {
            MainRow::Continue => self
                .continue_items
                .iter()
                .enumerate()
                .map(|(position, entry)| RowTile {
                    title: self
                        .continue_titles
                        .get(position)
                        .cloned()
                        .unwrap_or_default(),
                    detail: None,
                    marker: entry.marker(),
                    art: self
                        .continue_art
                        .get(position)
                        .map(|(url, frame)| self.art_or_frame(url.as_ref(), frame.clone())),
                })
                .collect(),
            MainRow::LatestMovies => self
                .movie_slot
                .loaded
                .as_ref()
                .map(|loaded| {
                    loaded
                        .latest
                        .iter()
                        .filter_map(|&item| Some((item, loaded.data.movies.get(item)?)))
                        .map(|(item, movie)| RowTile {
                            title: loaded.index.display_name(item, &movie.name).to_owned(),
                            detail: rating_detail(movie.rating.as_deref()),
                            marker: self.markers.movie(&movie.id),
                            art: Some(self.art_or_frame(
                                movie.icon.as_ref(),
                                super::frames::frame_key("movie", &movie.id),
                            )),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            MainRow::LatestSeries => self
                .series_slot
                .loaded
                .as_ref()
                .map(|loaded| {
                    loaded
                        .latest
                        .iter()
                        .filter_map(|&item| Some((item, loaded.data.series.get(item)?)))
                        .map(|(item, series)| RowTile {
                            title: loaded.index.display_name(item, &series.name).to_owned(),
                            detail: rating_detail(series.rating.as_deref()),
                            marker: self.markers.series(&series.id),
                            art: Some(self.art_or_frame(
                                series.cover.as_ref(),
                                super::frames::frame_key("series", &series.id),
                            )),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub(super) fn handle_main(&mut self, action: Action) {
        let rows = self.main_rows();
        if let Some(row) = self.main_row {
            if !rows.contains(&row) {
                self.main_row = None;
            }
        }
        if let Some(row) = self.main_row {
            return self.handle_main_row(row, &rows, action);
        }
        let index = self.main_focus.index();
        let columns = self.main_columns.max(1);
        let target = match action {
            Action::Left => index.checked_sub(1),
            Action::Right => Some(index + 1),
            Action::Up => index.checked_sub(columns),
            Action::Down => Some(index + columns),
            Action::Select => {
                self.activate_main_button(self.main_focus);
                return;
            }
            _ => return,
        };
        match target.and_then(|target| MainButton::ALL.get(target)) {
            Some(button) => self.main_focus = *button,
            None if action == Action::Down => self.main_row = rows.first().copied(),
            None => {}
        }
    }

    fn handle_main_row(&mut self, row: MainRow, rows: &[MainRow], action: Action) {
        let len = self.main_row_len(row);
        let selected = self.main_row_selection(row);
        let position = rows
            .iter()
            .position(|candidate| *candidate == row)
            .unwrap_or(0);
        match action {
            Action::Left => self.main_row_sel[row.slot()] = selected.saturating_sub(1),
            Action::Right => self.main_row_sel[row.slot()] = (selected + 1).min(len - 1),
            Action::Up if position == 0 => self.main_row = None,
            Action::Up => self.main_row = Some(rows[position - 1]),
            Action::Down => self.main_row = rows.get(position + 1).copied().or(Some(row)),
            Action::Back => self.main_row = None,
            Action::Select => self.open_main_tile(row, selected),
            _ => {}
        }
    }

    /// Open the details screen behind a Main tile.
    pub fn open_main_tile(&mut self, row: MainRow, tile: usize) {
        self.main_row = Some(row);
        self.main_row_sel[row.slot()] = tile;
        match row {
            MainRow::Continue => {
                let Some(entry) = self.continue_items.get(tile).cloned() else {
                    return;
                };
                self.open_continue_entry(&entry);
            }
            MainRow::LatestMovies => {
                let item = self
                    .movie_slot
                    .loaded
                    .as_ref()
                    .and_then(|l| l.latest.get(tile))
                    .copied();
                if let Some(item) = item {
                    self.open_movie_details(item);
                }
            }
            MainRow::LatestSeries => {
                let item = self
                    .series_slot
                    .loaded
                    .as_ref()
                    .and_then(|l| l.latest.get(tile))
                    .copied();
                if let Some(item) = item {
                    self.open_series_details(item);
                }
            }
        }
    }

    fn open_continue_entry(&mut self, entry: &ProgressEntry) {
        match entry.kind {
            ProgressKind::Movie => {
                let item = self
                    .movie_slot
                    .loaded
                    .as_ref()
                    .and_then(|loaded| loaded.index.position(&entry.id));
                match item {
                    Some(item) => self.open_movie_details(item),
                    None => self.notice = Some("That movie is not in the catalog.".into()),
                }
            }
            ProgressKind::Episode => {
                let (Some(series_id), Some(season), Some(episode)) =
                    (entry.series_id.clone(), entry.season, entry.episode)
                else {
                    return;
                };
                let from_catalog = self.series_slot.loaded.as_ref().and_then(|loaded| {
                    let item = loaded.index.position(&series_id)?;
                    loaded.data.series.get(item).cloned()
                });
                let series = from_catalog.unwrap_or_else(|| SeriesItem {
                    name: entry.series_name.clone().unwrap_or_default(),
                    id: series_id,
                    group_id: String::new(),
                    cover: None,
                    last_modified: None,
                    rating: None,
                });
                self.open_series(series, Some((season, episode)));
            }
        }
    }
    pub fn activate_main_button(&mut self, button: MainButton) {
        self.main_focus = button;
        match button {
            MainButton::LiveTv => self.enter_browser(BrowserId::Live),
            MainButton::Movies => self.enter_browser(BrowserId::Movies),
            MainButton::Series => self.enter_browser(BrowserId::Series),
            MainButton::Settings => self.enter_settings(),
            MainButton::Exit => self.exit_requested = true,
        }
    }
}
