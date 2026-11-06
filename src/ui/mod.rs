//! The island's drawing and behaviour, split by what it draws. Every file is a view or a part of one,
//! and adds its own `impl App` block; the shared drawing helpers are re-exported for all of them.

mod calendar;
mod clipboard;
mod clock;
mod closed_music;
mod companion;
mod events;
pub(crate) mod eyes;
mod icons;
mod input;
mod mirrors;
mod notice;
mod player;
mod prayer;
mod render;
mod shapes;
mod shelf;
mod stats;
mod tabs;
mod text;
mod timer;
mod todo;
mod tray;
mod water;
mod weather;
mod window;

pub(crate) use clipboard::*;
pub(crate) use clock::*;
pub(crate) use companion::*;
pub(crate) use events::*;
pub(crate) use icons::*;
pub(crate) use mirrors::Mirror;
pub(crate) use notice::*;
pub(crate) use player::*;
pub(crate) use prayer::PrayerTab;
pub(crate) use render::BodyKey;
pub(crate) use shapes::*;
pub(crate) use shelf::*;
pub(crate) use stats::StatsBackdrops;
pub(crate) use tabs::*;
pub(crate) use text::*;
pub(crate) use timer::*;
pub(crate) use todo::*;
pub(crate) use tray::*;
pub(crate) use weather::{WeatherTab, draw_moon};
pub(crate) use window::display_number;
#[cfg(test)]
pub(crate) use window::screen_by_order;
pub(crate) use window::*;
