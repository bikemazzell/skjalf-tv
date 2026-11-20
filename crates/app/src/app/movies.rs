//! Movie details: catalog data now, provider metadata when it arrives.

use skjalf_core::catalog::{
    format_duration, trailer_url, ContentKey, ContentKind, Movie, MovieInfo,
};
use skjalf_core::display::visual_text;
use skjalf_core::nav::View;
use skjalf_core::progress::ProgressKind;
use skjalf_core::xtream;

use super::catalog_jobs::JobTag;
use super::playback::OnDemand;
use super::App;
use crate::input::Action;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailInfo<T> {
    Loading,
    Ready(T),
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MovieButton {
    /// Continue from the saved position.
    Resume,
    /// Play, or play from the start when a resume position exists.
    #[default]
    Play,
    /// Open the YouTube trailer; only when the details have one.
    Trailer,
    Favorite,
}

#[derive(Debug, Clone)]
pub struct MovieDetails {
    pub movie: Movie,
    /// The title as drawn, converted once when the details open.
    pub display_name: String,
    pub info: DetailInfo<MovieInfo>,
    pub focus: MovieButton,
    request: u64,
}

impl App {
    pub(super) fn open_movie_details(&mut self, item: usize) {
        let Some(movie) = self
            .movie_slot
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.data.movies.get(item))
            .cloned()
        else {
            return;
        };
        self.details_request += 1;
        let request = self.details_request;
        self.request_movie_info(&movie.id, request);
        let focus = if self
            .resume_position(ProgressKind::Movie, &movie.id)
            .is_some()
        {
            MovieButton::Resume
        } else {
            MovieButton::Play
        };
        self.movie_details = Some(MovieDetails {
            display_name: visual_text(&movie.name).into_owned(),
            movie,
            info: DetailInfo::Loading,
            focus,
            request,
        });
        self.router.push(View::MovieDetails);
    }

    pub(super) fn apply_movie_info(
        &mut self,
        tag: JobTag,
        movie_id: &str,
        result: Result<MovieInfo, String>,
    ) {
        let current_source = self.source_id().map(str::to_owned);
        let Some(details) = &mut self.movie_details else {
            return;
        };
        if current_source.as_deref() != Some(tag.source_id.as_str())
            || details.request != tag.request
            || details.movie.id != movie_id
        {
            return;
        }
        details.info = match result {
            Ok(info) => DetailInfo::Ready(display_movie_info(info)),
            Err(_) => DetailInfo::Unavailable,
        };
    }

    pub fn movie_resume(&self) -> Option<f64> {
        let details = self.movie_details.as_ref()?;
        self.resume_position(ProgressKind::Movie, &details.movie.id)
    }

    /// Buttons in focus order; Resume appears only with a saved position,
    /// Trailer only when the details name one.
    pub fn movie_buttons(&self) -> Vec<MovieButton> {
        let mut buttons = Vec::with_capacity(4);
        if self.movie_resume().is_some() {
            buttons.push(MovieButton::Resume);
        }
        buttons.push(MovieButton::Play);
        if self.movie_trailer().is_some() {
            buttons.push(MovieButton::Trailer);
        }
        buttons.push(MovieButton::Favorite);
        buttons
    }

    fn movie_trailer(&self) -> Option<&str> {
        match &self.movie_details.as_ref()?.info {
            DetailInfo::Ready(info) => info.trailer.as_deref(),
            _ => None,
        }
    }

    fn open_movie_trailer(&mut self) {
        let Some(url) = self.movie_trailer().map(trailer_url) else {
            return;
        };
        let Some(opener) = &self.link_opener else {
            self.notice = Some("Trailers cannot open on this system.".into());
            return;
        };
        if let Err(error) = opener.open(&url) {
            self.notice = Some(format!("Could not open the trailer: {error}"));
        }
    }

    pub fn movie_button_label(&self, button: MovieButton) -> String {
        match (button, self.movie_resume()) {
            (MovieButton::Resume, Some(position)) => {
                format!("Resume {}", format_duration(position as u64))
            }
            (MovieButton::Resume, None) | (MovieButton::Play, None) => "Play".into(),
            (MovieButton::Play, Some(_)) => "Play from Start".into(),
            (MovieButton::Trailer, _) => "Trailer".into(),
            (MovieButton::Favorite, _) => {
                if self.movie_is_favorite() {
                    "Remove Favorite".into()
                } else {
                    "Favorite".into()
                }
            }
        }
    }

    /// The focused button, falling back to the first when it is gone.
    pub fn movie_focus(&self) -> MovieButton {
        let buttons = self.movie_buttons();
        let focus = self.movie_details.as_ref().map(|details| details.focus);
        focus
            .filter(|focus| buttons.contains(focus))
            .unwrap_or(buttons[0])
    }

    pub(super) fn handle_movie_details(&mut self, action: Action, now_ms: u64) {
        if self.movie_details.is_none() {
            self.router.back();
            return;
        }
        let buttons = self.movie_buttons();
        let focus = self.movie_focus();
        let index = buttons
            .iter()
            .position(|button| *button == focus)
            .unwrap_or(0);
        let move_to = |app: &mut Self, target: usize| {
            if let Some(details) = &mut app.movie_details {
                details.focus = buttons[target.min(buttons.len() - 1)];
            }
        };
        match action {
            Action::Left | Action::Up => move_to(self, index.saturating_sub(1)),
            Action::Right | Action::Down => move_to(self, index + 1),
            Action::Select => self.activate_movie_button(focus, now_ms),
            Action::Favorite => self.toggle_movie_favorite(now_ms),
            Action::Back => {
                self.router.back();
            }
            _ => {}
        }
    }

    pub fn activate_movie_button(&mut self, button: MovieButton, now_ms: u64) {
        if let Some(details) = &mut self.movie_details {
            details.focus = button;
        }
        match button {
            MovieButton::Resume => self.play_movie(true, now_ms),
            MovieButton::Play => self.play_movie(false, now_ms),
            MovieButton::Trailer => self.open_movie_trailer(),
            MovieButton::Favorite => self.toggle_movie_favorite(now_ms),
        }
    }

    pub fn movie_is_favorite(&self) -> bool {
        let (Some(details), Some(source_id)) = (&self.movie_details, self.source_id()) else {
            return false;
        };
        let key = ContentKey::new(ContentKind::Movie, &details.movie.id);
        self.favorites.contains(source_id, &key.favorite_key())
    }

    fn toggle_movie_favorite(&mut self, now_ms: u64) {
        if let Some(details) = &self.movie_details {
            let key = ContentKey::new(ContentKind::Movie, &details.movie.id);
            let name = details.movie.name.clone();
            self.toggle_favorite(&key, &name, now_ms);
        }
    }

    pub fn play_movie(&mut self, resume: bool, now_ms: u64) {
        let start = resume.then(|| self.movie_resume()).flatten();
        let (Some(details), Some(source)) = (&self.movie_details, &self.settings.source) else {
            return;
        };
        let Ok(url) = xtream::movie_url(source, &details.movie) else {
            return;
        };
        let key = ContentKey::new(ContentKind::Movie, &details.movie.id);
        let item = OnDemand {
            favorite: key.clone(),
            key,
            title: details.movie.name.clone(),
            record_title: details.movie.name.clone(),
            url,
            start,
            episode: None,
        };
        self.play_on_demand(item, now_ms);
    }
}

/// Convert the free-text fields once, when the info arrives.
fn display_movie_info(mut info: MovieInfo) -> MovieInfo {
    // The description stays logical: the view wraps it before reordering.
    for field in [&mut info.genre, &mut info.director, &mut info.release_date]
        .into_iter()
        .flatten()
    {
        *field = visual_text(field).into_owned();
    }
    info
}
