// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

pub fn now_text() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let sep = if tm.tm_sec % 2 == 0 { ':' } else { ' ' };
    format!("{:02}{sep}{:02}", tm.tm_hour, tm.tm_min)
}

/// The corner of the dashboard: the time, then the date, both as numbers so
/// they need no translation.
pub fn corner_text() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    corner(
        tm.tm_hour,
        tm.tm_min,
        tm.tm_mday,
        tm.tm_mon + 1,
        tm.tm_year + 1900,
    )
}

fn corner(hour: i32, minute: i32, day: i32, month: i32, year: i32) -> String {
    format!("{hour:02}:{minute:02} | {day:02}-{month:02}-{year:04}")
}

pub fn stamp() -> String {
    let mut tv: libc::timeval = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) };
    let now = tv.tv_sec;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        tv.tv_usec / 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corner_shows_time_then_date() {
        assert_eq!(corner(9, 5, 3, 4, 2026), "09:05 | 03-04-2026");
        assert_eq!(corner(23, 59, 31, 12, 2026), "23:59 | 31-12-2026");
    }
}
