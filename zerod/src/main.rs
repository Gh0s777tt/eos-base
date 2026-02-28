use redox_scheme::{scheme::SchemeState, RequestKind, SignalBehavior, Socket};

use scheme::ZeroScheme;

mod scheme;

enum Ty {
    Null,
    Zero,
}

fn main() {
    daemon::SchemeDaemon::new(daemon);
}

fn daemon(daemon: daemon::SchemeDaemon) -> ! {
    let ty = match &*std::env::args().next().unwrap() {
        "nulld" => Ty::Null,
        "zerod" => Ty::Zero,
        _ => panic!("needs to be called as either nulld or zerod"),
    };

    let socket = Socket::create().expect("zerod: failed to create zero scheme");
    let mut state = SchemeState::new();
    let mut zero_scheme = ZeroScheme(ty);

    let _ = daemon.ready_sync_scheme(&socket, &mut zero_scheme);

    libredox::call::setrens(0, 0).expect("zerod: failed to enter null namespace");

    loop {
        let Some(request) = socket
            .next_request(SignalBehavior::Restart)
            .expect("zerod: failed to read events from zero scheme")
        else {
            std::process::exit(0);
        };
        match request.kind() {
            RequestKind::Call(request) => {
                let response = request.handle_sync(&mut zero_scheme, &mut state);

                socket
                    .write_response(response, SignalBehavior::Restart)
                    .expect("zerod: failed to write responses to zero scheme");
            }
            _ => (),
        }
    }
}
