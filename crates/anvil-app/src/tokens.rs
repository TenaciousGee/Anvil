// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Design tokens exposed to QML as the `Tokens` singleton.
//!
//! Defined in Rust rather than as a QML singleton file so the values have a
//! single source that settings (light theme, accent colour, font scale) can
//! change at runtime. Colours are `#RRGGBB` or `#AARRGGBB` strings, which
//! QML converts to `color` on assignment.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, bg)]
        #[qproperty(QString, rail)]
        #[qproperty(QString, surface)]
        #[qproperty(QString, raised)]
        #[qproperty(QString, border)]
        #[qproperty(QString, text)]
        #[qproperty(QString, dim)]
        #[qproperty(QString, accent)]
        #[qproperty(QString, ink)]
        #[qproperty(QString, tint)]
        #[qproperty(QString, warn)]
        #[qproperty(QString, caution)]
        #[qproperty(QString, good)]
        #[qproperty(QString, mono)]
        #[qproperty(i32, radius)]
        #[qproperty(i32, gap)]
        #[qproperty(i32, body)]
        #[qproperty(i32, heading)]
        #[qproperty(i32, hero)]
        type Tokens = super::TokensRust;
    }
}

use cxx_qt_lib::QString;

pub struct TokensRust {
    bg: QString,
    rail: QString,
    surface: QString,
    /// Raised surface: rows inside cards, selected segments.
    raised: QString,
    border: QString,
    text: QString,
    /// Secondary text.
    dim: QString,
    accent: QString,
    /// Text drawn on top of the accent colour.
    ink: QString,
    /// Accent at low opacity, for selected and drag-over states.
    tint: QString,
    warn: QString,
    /// Warning colour at low opacity, for warning cards.
    caution: QString,
    good: QString,
    mono: QString,
    radius: i32,
    gap: i32,
    body: i32,
    heading: i32,
    hero: i32,
}

impl Default for TokensRust {
    fn default() -> Self {
        let s = |v: &str| QString::from(v);
        TokensRust {
            bg: s("#0E1014"),
            rail: s("#0A0C0F"),
            surface: s("#161A21"),
            raised: s("#1E232D"),
            border: s("#262C37"),
            text: s("#E8ECF2"),
            dim: s("#98A1B0"),
            accent: s("#7C9CFF"),
            ink: s("#0E1014"),
            tint: s("#247C9CFF"),
            warn: s("#F2B35B"),
            caution: s("#1FF2B35B"),
            good: s("#5CCB8F"),
            mono: s("monospace"),
            radius: 8,
            gap: 16,
            body: 13,
            heading: 15,
            hero: 22,
        }
    }
}
