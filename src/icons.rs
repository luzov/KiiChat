//! The asset source for this app.
//!
//! Everything comes from the bundled component icons except two glyphs the
//! component set does not ship: a branch and a pencil. GPUI rasterizes SVGs as
//! alpha masks and tints them with the element's text color, so the embedded
//! `stroke="currentColor"` in these two is irrelevant to how they end up
//! looking — they take the theme's ink like every other icon.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Lucide `git-branch` — stands in for "branch this conversation".
///
/// Drawn from a 24px grid at stroke-width 2.2 rather than the upstream 2: at
/// the 24px the component draws icon buttons, a 2px stroke over three shapes
/// reads as a smudge, and three open circles blur into one another.
const GIT_BRANCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6a9 9 0 0 0-9 9V3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/></svg>"#;

/// Lucide `square-pen` — edit a message in place.
const SQUARE_PEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/></svg>"#;

pub const GIT_BRANCH_PATH: &str = "icons/kiichat-git-branch.svg";
pub const SQUARE_PEN_PATH: &str = "icons/kiichat-square-pen.svg";

/// The bundled component assets plus [`GIT_BRANCH_PATH`] and [`SQUARE_PEN_PATH`].
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let extra = match path {
            GIT_BRANCH_PATH => Some(GIT_BRANCH),
            SQUARE_PEN_PATH => Some(SQUARE_PEN),
            _ => None,
        };
        if let Some(svg) = extra {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        for extra in [GIT_BRANCH_PATH, SQUARE_PEN_PATH] {
            if extra.starts_with(path) {
                paths.push(extra.into());
            }
        }
        Ok(paths)
    }
}