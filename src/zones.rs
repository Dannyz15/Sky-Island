//! The zones of JAKIM's e-Solat (Malaysia). Its prayer times are given for a zone, not for a point, so a city has to be
//! matched with one. Each zone has a few towns in it (its points); the zone nearest to the city wins.

use crate::weather::Place;

/// A zone: its code, a short name, and a few towns in it (latitude, longitude).
pub struct Zone {
    pub code: &'static str,
    pub label: &'static str,
    pub points: &'static [(f64, f64)],
}

const fn z(code: &'static str, label: &'static str, points: &'static [(f64, f64)]) -> Zone {
    Zone { code, label, points }
}

/// JAKIM's 60 zones, in the order of their codes.
pub const ZONES: &[Zone] = &[
    z("JHR01", "Pulau Aur, Pemanggil", &[(2.45, 104.52)]),
    z("JHR02", "Johor Bahru, Kota Tinggi", &[(1.4927, 103.7414), (1.7381, 103.8999), (2.4312, 103.8405), (1.6561, 103.6032)]),
    z("JHR03", "Kluang, Pontian", &[(2.0251, 103.3184), (1.4866, 103.3893)]),
    z("JHR04", "Batu Pahat, Muar, Segamat", &[(1.8548, 102.9325), (2.0442, 102.5689), (2.5148, 102.8158), (2.2673, 102.5455)]),
    z("KDH01", "Kota Setar, Kubang Pasu", &[(6.1248, 100.3678), (6.2667, 100.4217)]),
    z("KDH02", "Kuala Muda, Yan, Pendang", &[(5.647, 100.4877), (5.8, 100.35), (5.9961, 100.5)]),
    z("KDH03", "Padang Terap, Sik", &[(6.2625, 100.6079), (5.8176, 100.7355)]),
    z("KDH04", "Baling", &[(5.6767, 100.9167)]),
    z("KDH05", "Kulim, Bandar Baharu", &[(5.365, 100.5614), (5.1333, 100.5)]),
    z("KDH06", "Langkawi", &[(6.35, 99.8)]),
    z("KDH07", "Gunung Jerai", &[(5.79, 100.43)]),
    z("KTN01", "Kota Bharu, Pasir Mas", &[(6.1254, 102.2381), (5.5303, 102.2009), (5.8087, 102.1494), (5.7667, 102.2167)]),
    z("KTN02", "Gua Musang, Jeli", &[(4.8822, 101.9645), (5.7, 101.8333)]),
    z("MLK01", "Melaka", &[(2.1896, 102.2501), (2.3808, 102.2092), (2.3167, 102.4333)]),
    z("NGS01", "Tampin, Jempol", &[(2.4706, 102.2306), (2.8167, 102.4)]),
    z("NGS02", "Kuala Pilah, Jelebu, Rembau", &[(2.7389, 102.2494), (3.1, 102.0667), (2.5858, 102.0961)]),
    z("NGS03", "Seremban, Port Dickson", &[(2.7258, 101.9424), (2.5225, 101.7963)]),
    z("PHG01", "Pulau Tioman", &[(2.8167, 104.1667)]),
    z("PHG02", "Kuantan, Pekan", &[(3.8077, 103.326), (3.4833, 103.4), (3.0504, 103.0828)]),
    z("PHG03", "Temerloh, Jerantut, Maran", &[(3.4506, 102.4184), (3.9333, 102.3667), (3.5833, 102.7667)]),
    z("PHG04", "Bentong, Raub, Lipis", &[(3.5222, 101.9086), (3.7936, 101.8572), (4.1833, 102.05)]),
    z("PHG05", "Genting Sempah, Janda Baik", &[(3.3667, 101.7833), (3.3333, 101.8667)]),
    z("PHG06", "Cameron, Genting Highlands", &[(4.4721, 101.3802), (3.4236, 101.7937), (3.7167, 101.7333)]),
    z("PHG07", "Rompin, Endau", &[(2.7167, 103.4833), (2.65, 103.6167)]),
    z("PRK01", "Tapah, Tanjung Malim", &[(4.1969, 101.2609), (3.8333, 101.4), (3.6833, 101.5167)]),
    z("PRK02", "Ipoh, Kuala Kangsar, Kampar", &[(4.5975, 101.0901), (4.7667, 100.9333), (4.4722, 101.0447), (4.3, 101.15)]),
    z("PRK03", "Grik, Lenggong", &[(5.4333, 101.1333), (5.1, 100.9833), (5.7, 101.0)]),
    z("PRK04", "Temengor, Belum", &[(5.5333, 101.4167), (5.7, 101.4)]),
    z("PRK05", "Teluk Intan, Sitiawan, Lumut", &[(4.0259, 101.0213), (4.2167, 100.7), (4.2333, 100.6333), (3.9833, 100.7667), (4.3667, 100.9667)]),
    z("PRK06", "Taiping, Parit Buntar", &[(4.85, 100.7333), (5.1167, 100.5), (5.0167, 100.5333), (5.2167, 100.7)]),
    z("PRK07", "Bukit Larut", &[(4.8667, 100.7967)]),
    z("PLS01", "Perlis", &[(6.4414, 100.1986)]),
    z("PNG01", "Pulau Pinang", &[(5.4141, 100.3288), (5.3, 100.2667), (5.4, 100.3667)]),
    z("SBH01", "Sandakan", &[(5.8402, 118.1179), (5.5, 118.27)]),
    z("SBH02", "Beluran, Telupid", &[(5.8833, 117.55), (5.5333, 117.1167)]),
    z("SBH03", "Lahad Datu, Semporna", &[(5.0265, 118.3269), (4.6833, 118.2167), (4.4833, 118.6167)]),
    z("SBH04", "Tawau", &[(4.2441, 117.8912), (4.4167, 117.4333)]),
    z("SBH05", "Kudat, Kota Marudu", &[(6.8833, 116.8333), (6.5, 116.7333), (6.7, 117.0667)]),
    z("SBH06", "Gunung Kinabalu", &[(6.0753, 116.5583)]),
    z("SBH07", "Kota Kinabalu, Ranau", &[(5.9804, 116.0735), (5.9567, 116.6667), (6.35, 116.4333), (5.7333, 115.9333), (6.1833, 116.2333)]),
    z("SBH08", "Keningau, Tambunan", &[(5.3378, 116.1602), (5.6833, 116.3667), (4.35, 116.7)]),
    z("SBH09", "Beaufort, Sipitang, Tenom", &[(5.3475, 115.7439), (5.0833, 115.55), (5.1333, 115.95), (5.5833, 115.6)]),
    z("SWK01", "Limbang, Lawas", &[(4.75, 115.0), (4.85, 115.4)]),
    z("SWK02", "Miri, Marudi", &[(4.4, 113.9833), (4.1833, 114.3167)]),
    z("SWK03", "Bintulu, Belaga", &[(3.1667, 113.0333), (2.7, 113.7667)]),
    z("SWK04", "Sibu, Mukah, Kapit", &[(2.3, 111.8167), (2.9, 112.0833), (2.0167, 112.9333)]),
    z("SWK05", "Sarikei, Matu", &[(2.1167, 111.5167), (2.6333, 111.5), (2.1667, 111.6167)]),
    z("SWK06", "Sri Aman, Betong, Saratok", &[(1.2333, 111.4667), (1.4167, 111.5167), (1.7667, 111.3667), (1.05, 111.8333)]),
    z("SWK07", "Serian, Samarahan", &[(1.1667, 110.5667), (1.4583, 110.4833), (1.4, 110.7667)]),
    z("SWK08", "Kuching, Bau, Lundu", &[(1.5535, 110.3593), (1.4167, 110.15), (1.6667, 109.8667)]),
    // A special zone for one village; it is chosen by hand, never by distance.
    z("SWK09", "Kampung Patarikan", &[]),
    z("SGR01", "Shah Alam, Petaling, Gombak", &[(3.0733, 101.5185), (3.1073, 101.6067), (3.2639, 101.6988), (2.6903, 101.7497), (2.9935, 101.7874), (3.5667, 101.65), (3.3214, 101.5767)]),
    z("SGR02", "Kuala Selangor", &[(3.34, 101.25), (3.7667, 100.9833)]),
    z("SGR03", "Klang, Kuala Langat", &[(3.0449, 101.4456), (2.8167, 101.5)]),
    z("TRG01", "Kuala Terengganu, Marang", &[(5.3302, 103.1408), (5.2, 103.2)]),
    z("TRG02", "Besut, Setiu", &[(5.8333, 102.5667), (5.5333, 102.7167)]),
    z("TRG03", "Hulu Terengganu", &[(5.0667, 103.0)]),
    z("TRG04", "Dungun, Kemaman", &[(4.7554, 103.4144), (4.2333, 103.4333)]),
    z("WLY01", "Kuala Lumpur, Putrajaya", &[(3.139, 101.6869), (2.9264, 101.6964)]),
    z("WLY02", "Labuan", &[(5.2831, 115.2308)]),
];

/// What the settings put before the zone that was found by distance.
pub const AUTOMATIC: &str = "Auto: ";

/// A city further than this from every town of every zone is not in Malaysia as far as e-Solat is concerned.
const FAR_KM: f64 = 120.0;

/// How a zone is shown in the settings: its code, a dot, and its towns.
pub fn describe(zone: &Zone) -> String {
    format!("{} \u{00B7} {}", zone.code, zone.label)
}

/// The zone with this code.
pub fn find(code: &str) -> Option<&'static Zone> {
    ZONES.iter().find(|z| z.code == code)
}

/// Where a zone stands in `ZONES`.
pub fn index_of(code: &str) -> Option<usize> {
    ZONES.iter().position(|z| z.code == code)
}

/// The distance in kilometres between two points, near enough for a few hundred kilometres.
fn km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dy = (a.0 - b.0) * 111.2;
    let dx = (a.1 - b.1) * 111.2 * ((a.0 + b.0) / 2.0).to_radians().cos();
    dx.hypot(dy)
}

/// The zone nearest to a city. `None` for a city that is not in Malaysia (another country, or no town of any zone within
/// `FAR_KM`).
pub fn nearest(place: &Place) -> Option<&'static Zone> {
    if !place.cc.is_empty() && !place.cc.eq_ignore_ascii_case("MY") {
        return None;
    }
    let here = (place.lat, place.lon);
    ZONES
        .iter()
        .flat_map(|zone| zone.points.iter().map(move |p| (zone, km(here, *p))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d <= FAR_KM)
        .map(|(zone, _)| zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn city(lat: f64, lon: f64, cc: &str) -> Place {
        Place { name: "x".into(), lat, lon, tz: String::new(), cc: cc.into() }
    }

    fn code(p: &Place) -> Option<&'static str> {
        nearest(p).map(|z| z.code)
    }

    #[test]
    fn the_codes_are_unique_and_all_sixty_are_listed() {
        assert_eq!(ZONES.len(), 60);
        for (i, zone) in ZONES.iter().enumerate() {
            assert!(!zone.label.is_empty());
            assert!(zone.code.len() == 5 && zone.code[3..].chars().all(|c| c.is_ascii_digit()), "{}", zone.code);
            assert!(!ZONES[i + 1..].iter().any(|o| o.code == zone.code), "{} twice", zone.code);
            assert!(zone.points.iter().all(|(la, lo)| (0.0..8.0).contains(la) && (98.0..120.0).contains(lo)), "{}", zone.code);
        }
        assert_eq!(index_of("WLY01"), Some(58));
        assert!(find("XXX00").is_none());
    }

    #[test]
    fn a_city_gets_the_zone_nearest_to_it() {
        assert_eq!(code(&city(3.1412, 101.69, "MY")), Some("WLY01"), "Kuala Lumpur");
        assert_eq!(code(&city(3.0733, 101.5185, "MY")), Some("SGR01"), "Shah Alam");
        assert_eq!(code(&city(3.0449, 101.4456, "MY")), Some("SGR03"), "Klang");
        assert_eq!(code(&city(1.4927, 103.7414, "MY")), Some("JHR02"), "Johor Bahru");
        assert_eq!(code(&city(5.4141, 100.3288, "MY")), Some("PNG01"), "George Town");
        assert_eq!(code(&city(1.5535, 110.3593, "MY")), Some("SWK08"), "Kuching");
        assert_eq!(code(&city(5.9804, 116.0735, "MY")), Some("SBH07"), "Kota Kinabalu");
        assert_eq!(code(&city(6.1254, 102.2381, "")), Some("KTN01"), "an unknown country is judged by distance");
    }

    #[test]
    fn a_city_outside_malaysia_has_no_zone() {
        assert_eq!(code(&city(1.3521, 103.8198, "SG")), None, "another country");
        assert_eq!(code(&city(-6.2, 106.8, "")), None, "Jakarta is far from every zone");
        assert_eq!(code(&city(51.5, -0.12, "GB")), None);
    }
}
