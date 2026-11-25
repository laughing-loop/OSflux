//! Which of Slint's renderers draws Studio's window.
//!
//! On Windows, Studio uses Slint's software renderer. Tested on Windows 10,
//! its window was ready in about 110 ms, against about 1,230 ms for the
//! OpenGL renderer (femtovg), which has to start OpenGL first, and Studio's
//! flat, square screens need nothing a GPU adds. Elsewhere Slint's own
//! default stays.
//!
//! `SLINT_BACKEND` still decides when it is set, for example
//! `SLINT_BACKEND=winit-femtovg` to use OpenGL.

/// The renderer Studio asks Slint for, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// `SLINT_BACKEND` names the backend and renderer; Slint reads it.
    FromEnvironment(String),
    /// Studio's default on this platform: the winit backend with this renderer.
    Default(&'static str),
    /// Slint's own default.
    SlintDefault,
}

/// What to use, given `SLINT_BACKEND` and whether this is Windows.
pub fn choose(slint_backend: Option<&str>, windows: bool) -> Choice {
    match slint_backend
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => Choice::FromEnvironment(value.to_string()),
        None if windows => Choice::Default("software"),
        None => Choice::SlintDefault,
    }
}

/// Picks the renderer before the first window is created, and logs which.
/// When the software renderer cannot be selected, Slint's default is used.
pub fn select() -> Choice {
    let choice = choose(
        std::env::var("SLINT_BACKEND").ok().as_deref(),
        cfg!(windows),
    );
    match &choice {
        Choice::FromEnvironment(value) => {
            crate::log!("renderer: {value}, as SLINT_BACKEND asks");
        }
        Choice::Default(renderer) => {
            let selected = slint::BackendSelector::new()
                .backend_name("winit".to_string())
                .renderer_name(renderer.to_string())
                .select();
            match selected {
                Ok(()) => crate::log!(
                    "renderer: winit-{renderer}, Studio's default (SLINT_BACKEND=winit-femtovg for OpenGL)"
                ),
                Err(error) => {
                    crate::log!("renderer: winit-{renderer} unavailable ({error}); Slint's default");
                    return Choice::SlintDefault;
                }
            }
        }
        Choice::SlintDefault => {
            crate::log!("renderer: Slint's default (femtovg, or software without OpenGL)");
        }
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::{choose, Choice};

    #[test]
    fn windows_uses_the_software_renderer_unless_told_otherwise() {
        assert_eq!(choose(None, true), Choice::Default("software"));
        assert_eq!(choose(Some(""), true), Choice::Default("software"));
        assert_eq!(
            choose(Some("winit-femtovg"), true),
            Choice::FromEnvironment("winit-femtovg".to_string())
        );
    }

    #[test]
    fn elsewhere_slint_decides_unless_told_otherwise() {
        assert_eq!(choose(None, false), Choice::SlintDefault);
        assert_eq!(
            choose(Some(" winit-software "), false),
            Choice::FromEnvironment("winit-software".to_string())
        );
    }
}
