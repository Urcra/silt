//! adb for one device (ADR 0093): the sandbox's adb reaches this machine's
//! adb server through a filter that lets through only what names the one
//! device relayed to it, so the agent never sees or drives another.

/// What to do with the first request of a client's connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Send this, maybe rewritten to name the device, then pass the rest
    /// of the connection through.
    Pass(String),
    /// Send it, and keep only the device's line of the list that comes back.
    Devices(String),
    Refuse(String),
}

/// How the server answers a host request that only asks about it.
const HOST_ONLY: [&str; 4] = ["host:version", "host:features", "host:host-features", "host:check-features"];

/// Decides `request`, the payload of a client's first message.
pub fn decide(request: &str, serial: &str) -> Decision {
    if HOST_ONLY.contains(&request) {
        return Decision::Pass(request.to_owned());
    }
    match request {
        "host:devices" | "host:devices-l" => return Decision::Devices(request.to_owned()),
        // Whichever device it would take is the one relayed.
        "host:transport-any" | "host:transport-usb" | "host:transport-local" => {
            return Decision::Pass(format!("host:transport:{serial}"));
        }
        "host:tport:any" | "host:tport:usb" | "host:tport:local" => return Decision::Pass(format!("host:tport:serial:{serial}")),
        _ => {}
    }
    let named = request
        .strip_prefix("host:transport:")
        .or_else(|| request.strip_prefix("host:tport:serial:"))
        .map(|named| (named, None))
        .or_else(|| {
            let rest = request.strip_prefix("host-serial:")?;
            let (named, then) = rest.split_once(':')?;
            Some((named, Some(then)))
        });
    match named {
        Some((named, then)) if named == serial => match then {
            // Questions about the device only, never what changes the server.
            Some(then) if !then.starts_with("forward") && !then.starts_with("reverse") && !then.starts_with("killforward") => {
                Decision::Pass(request.to_owned())
            }
            Some(then) => Decision::Refuse(format!("{then} isn't relayed to the sandbox")),
            None => Decision::Pass(request.to_owned()),
        },
        Some((named, _)) => Decision::Refuse(format!("only {serial} is relayed to this sandbox, not {named}")),
        None => Decision::Refuse(format!("{request} isn't relayed to the sandbox, which reaches {serial} only")),
    }
}

/// adb's framing: four hex digits of length, then the payload.
pub fn frame(payload: &str) -> Vec<u8> {
    format!("{:04x}{payload}", payload.len()).into_bytes()
}

/// A refusal as the adb client shows it.
pub fn fail(message: &str) -> Vec<u8> {
    let mut out = b"FAIL".to_vec();
    out.extend(frame(message));
    out
}

/// The device list with only `serial`'s line.
pub fn only(list: &str, serial: &str) -> String {
    list.lines().filter(|line| line.split_whitespace().next() == Some(serial)).map(|line| format!("{line}\n")).collect()
}

/// The length a frame's four hex digits say, if they're hex.
pub fn length(header: &[u8; 4]) -> Option<usize> {
    usize::from_str_radix(std::str::from_utf8(header).ok()?, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_relayed_device_is_reached() {
        assert_eq!(decide("host:version", "R58M"), Decision::Pass("host:version".into()));
        assert_eq!(decide("host:transport:R58M", "R58M"), Decision::Pass("host:transport:R58M".into()));
        assert_eq!(decide("host:transport-any", "R58M"), Decision::Pass("host:transport:R58M".into()));
        assert_eq!(decide("host:tport:any", "R58M"), Decision::Pass("host:tport:serial:R58M".into()));
        assert_eq!(decide("host-serial:R58M:get-state", "R58M"), Decision::Pass("host-serial:R58M:get-state".into()));
        assert!(matches!(decide("host:transport:emulator-5554", "R58M"), Decision::Refuse(_)));
        assert!(matches!(decide("host-serial:other:get-state", "R58M"), Decision::Refuse(_)));
        assert!(matches!(decide("host-serial:R58M:forward:tcp:1;tcp:2", "R58M"), Decision::Refuse(_)));
        for request in
            ["host:kill", "host:connect:10.0.0.5:5555", "host:track-devices", "host:disconnect:", "host:transport:R58M2"]
        {
            assert!(matches!(decide(request, "R58M"), Decision::Refuse(_)), "{request}");
        }
    }

    #[test]
    fn the_list_shows_the_one_device() {
        let list = "R58M\tdevice\nemulator-5554\tdevice\nR58M2\tunauthorized\n";
        assert_eq!(only(list, "R58M"), "R58M\tdevice\n");
        assert_eq!(frame("host:version"), b"000chost:version".to_vec());
        assert_eq!(length(b"000c"), Some(12));
        assert_eq!(fail("no"), b"FAIL0002no".to_vec());
    }
}
