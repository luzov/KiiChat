//! The asset source for this app.
//!
//! Everything comes from the bundled component icons except two glyphs the
//! component set does not ship: a branch and a pencil. GPUI rasterizes SVGs as
//! alpha masks and tints them with the element's text color, so the embedded
//! `stroke="currentColor"` in these two is irrelevant to how they end up
//! looking — they take the theme's ink like every other icon.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Lucide `git-fork` — stands in for "branch this conversation".
const GIT_FORK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/></svg>"#;

/// Lucide `pencil` — edit a message in place.
const PENCIL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/></svg>"#;

pub const GIT_FORK_PATH: &str = "icons/kiichat-git-fork.svg";
pub const PENCIL_PATH: &str = "icons/kiichat-pencil.svg";

/// The bundled component assets plus [`GIT_FORK_PATH`] and [`PENCIL_PATH`].
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let extra = match path {
            GIT_FORK_PATH => Some(GIT_FORK),
            PENCIL_PATH => Some(PENCIL),
            _ => None,
        };
        if let Some(svg) = extra {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        for extra in [GIT_FORK_PATH, PENCIL_PATH] {
            if extra.starts_with(path) {
                paths.push(extra.into());
            }
        }
        Ok(paths)
    }
}