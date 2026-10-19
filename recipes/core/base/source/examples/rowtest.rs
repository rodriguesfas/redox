fn main() {
    let sessions = logind::session::Sessions::from_utmp(&[
        logind::utmp::UtmpRecord::user_process(42, "tty1", "user", 1_792_022_400),
        logind::utmp::UtmpRecord::user_process(77, "tty2", "root", 1_792_022_500),
    ]);
    print!("{}", logind::render::render_sessions(sessions.list(), false));
    print!("{}", logind::render::render_seats(&sessions.seats(), false));
    print!("{}", logind::render::render_session_status(sessions.session("1").unwrap()));
    print!("{}", logind::render::render_seat_status(&sessions.seats()[0], &sessions));
}
