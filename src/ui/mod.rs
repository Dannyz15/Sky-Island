//! The island's drawing and behaviour, split by what it draws. Every file is a view or a part of one,
//! and adds its own `impl App` block; the shared drawing helpers are re-exported for all of them.

mod calendar;
mod clipboard;
mod clock;
mod companion;
mod events;
mod icons;
mod input;
mod notice;
mod player;
mod render;
mod shapes;
mod shelf;
mod tabs;
mod text;
mod timer;
mod todo;
mod water;
mod window;

pub(crate) use clock::*;
pub(crate) use icons::*;
pub(crate) use player::*;
pub(crate) use shapes::*;
pub(crate) use text::*;
pub(crate) use events::*;
pub(crate) use clipboard::*;
pub(crate) use companion::*;
pub(crate) use notice::*;
pub(crate) use shelf::*;
pub(crate) use timer::*;
pub(crate) use todo::*;
pub(crate) use window::*;
pub(crate) use tabs::*;
