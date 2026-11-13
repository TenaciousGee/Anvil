// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! HDR10 static metadata: parsing from ffprobe and formatting for encoders.

use serde::{Deserialize, Serialize};

/// CIE 1931 xy chromaticity coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Chromaticity {
    pub x: f64,
    pub y: f64,
}

/// SMPTE ST 2086 mastering display colour volume.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MasteringDisplay {
    pub red: Chromaticity,
    pub green: Chromaticity,
    pub blue: Chromaticity,
    pub white_point: Chromaticity,
    /// cd/m²
    pub max_luminance: f64,
    /// cd/m²
    pub min_luminance: f64,
}

/// MaxCLL / MaxFALL in cd/m².
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentLightLevel {
    pub max_cll: u32,
    pub max_fall: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Hdr10Metadata {
    pub mastering_display: Option<MasteringDisplay>,
    pub content_light: Option<ContentLightLevel>,
}

impl Hdr10Metadata {
    pub fn is_empty(&self) -> bool {
        self.mastering_display.is_none() && self.content_light.is_none()
    }
}

/// Parse an ffprobe rational string such as `"34000/50000"` or a plain number.
pub fn parse_ratio(s: &str) -> Option<f64> {
    match s.split_once('/') {
        Some((n, d)) => {
            let n: f64 = n.trim().parse().ok()?;
            let d: f64 = d.trim().parse().ok()?;
            if d == 0.0 {
                None
            } else {
                Some(n / d)
            }
        }
        None => s.trim().parse().ok(),
    }
}

impl MasteringDisplay {
    /// x265 `--master-display` value. Chromaticities are in units of 0.00002
    /// and luminance in units of 0.0001 cd/m², ordered G, B, R, WP, L(max,min).
    pub fn to_x265(&self) -> String {
        let c = |v: f64| (v * 50_000.0).round() as u64;
        let l = |v: f64| (v * 10_000.0).round() as u64;
        format!(
            "G({},{})B({},{})R({},{})WP({},{})L({},{})",
            c(self.green.x),
            c(self.green.y),
            c(self.blue.x),
            c(self.blue.y),
            c(self.red.x),
            c(self.red.y),
            c(self.white_point.x),
            c(self.white_point.y),
            l(self.max_luminance),
            l(self.min_luminance),
        )
    }

    /// SVT-AV1 `--mastering-display` value, using decimal coordinates.
    pub fn to_svt_av1(&self) -> String {
        format!(
            "G({:.4},{:.4})B({:.4},{:.4})R({:.4},{:.4})WP({:.4},{:.4})L({:.4},{:.4})",
            self.green.x,
            self.green.y,
            self.blue.x,
            self.blue.y,
            self.red.x,
            self.red.y,
            self.white_point.x,
            self.white_point.y,
            self.max_luminance,
            self.min_luminance,
        )
    }
}

impl ContentLightLevel {
    /// `"MaxCLL,MaxFALL"`, the format both x265 and SVT-AV1 accept.
    pub fn to_arg(&self) -> String {
        format!("{},{}", self.max_cll, self.max_fall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p3_d65_1000nit() -> MasteringDisplay {
        MasteringDisplay {
            red: Chromaticity { x: 0.68, y: 0.32 },
            green: Chromaticity { x: 0.265, y: 0.69 },
            blue: Chromaticity { x: 0.15, y: 0.06 },
            white_point: Chromaticity {
                x: 0.3127,
                y: 0.329,
            },
            max_luminance: 1000.0,
            min_luminance: 0.0001,
        }
    }

    #[test]
    fn x265_master_display_matches_known_string() {
        // The common Display P3 / 1000-nit string seen on UHD Blu-ray.
        assert_eq!(
            p3_d65_1000nit().to_x265(),
            "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)"
        );
    }

    #[test]
    fn svt_master_display_format() {
        assert_eq!(
            p3_d65_1000nit().to_svt_av1(),
            "G(0.2650,0.6900)B(0.1500,0.0600)R(0.6800,0.3200)WP(0.3127,0.3290)L(1000.0000,0.0001)"
        );
    }

    #[test]
    fn ratios() {
        assert_eq!(parse_ratio("34000/50000"), Some(0.68));
        assert_eq!(parse_ratio("10000000/10000"), Some(1000.0));
        assert_eq!(parse_ratio("1/0"), None);
        assert_eq!(parse_ratio("0.5"), Some(0.5));
    }
}
