use redox_scheme::{
    scheme::{SchemeState, SchemeSync},
    RequestKind, Response, SignalBehavior, Socket,
};
use std::process;

use crate::scheme::LogScheme;

mod scheme;

fn daemon(daemon: daemon::SchemeDaemon) -> ! {
    let socket = Socket::create().expect("logd: failed to create log scheme");

    let mut state = SchemeState::new();
    let mut scheme = LogScheme::new(&socket);

    let _ = daemon.ready_sync_scheme(&socket, &mut scheme);

    libredox::call::setrens(0, 0).expect("logd: failed to enter null namespace");

    while let Some(request) = socket
        .next_request(SignalBehavior::Restart)
        .expect("logd: failed to read events from log scheme")
    {
        let request = match request.kind() {
            RequestKind::Call(call) => call,
            RequestKind::OnClose { id } => {
                scheme.on_close(id);
                continue;
            }
            RequestKind::SendFd(sendfd_request) => {
                let result = scheme.on_sendfd(&sendfd_request);
                let resp = Response::new(result, sendfd_request);
                socket
                    .write_response(resp, SignalBehavior::Restart)
                    .expect("logd: failed to write responses to log scheme");
                continue;
            }
            _ => continue,
        };

        let response = request.handle_sync(&mut scheme, &mut state);
        socket
            .write_response(response, SignalBehavior::Restart)
            .expect("logd: failed to write responses to log scheme");
    }
    process::exit(0);
}

fn main() {
    daemon::SchemeDaemon::new(daemon);
}
