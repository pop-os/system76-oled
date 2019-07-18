use env_logger::Env;
use inotify::{
    Inotify,
    WatchMask,
};
use log::{debug, info, trace};
use std::{fs, process};
use std::fmt::Write;
use std::io::{Error, Read, Seek, SeekFrom};
use std::os::unix::io::AsRawFd;

fn main() {
    env_logger::from_env(Env::default().default_filter_or("info")).init();

    let vendor = fs::read_to_string("/sys/class/dmi/id/sys_vendor")
        .unwrap_or(String::new());
    let vendor = vendor.trim();
    let model = fs::read_to_string("/sys/class/dmi/id/product_version")
        .unwrap_or(String::new());
    let model = model.trim();

    let output_opt = match (vendor, model) {
        ("System76", "addw1") => Some("eDP-1"),
        _ => None,
    };

    let output = if let Some(output) = output_opt {
        info!("Vendor '{}' Model '{}' has OLED display on '{}'", vendor, model, output);
        output
    } else {
        debug!("Vendor '{}' Model '{}' does not have OLED display", vendor, model);
        process::exit(0);
    };

    let mut inotify = Inotify::init()
        .expect("failed to initialize inotify");

    let requested_path = "/sys/class/backlight/intel_backlight/brightness";

    let requested_watch = inotify.add_watch(requested_path, WatchMask::MODIFY)
        .expect("failed to watch requested brightness");

    let mut requested_file = fs::File::open(requested_path)
        .expect("failed to open requested brightness");

    let max_path = "/sys/class/backlight/intel_backlight/max_brightness";

    let max_watch = inotify.add_watch(max_path, WatchMask::MODIFY)
        .expect("failed to watch max brightness");

    let mut max_file = fs::File::open(max_path)
        .expect("failed to open max brightness");

    let mut requested_update = true;
    let mut requested_str = String::with_capacity(256);
    let mut requested = 0;
    let mut max_update = true;
    let mut max_str = String::with_capacity(256);
    let mut max = 0;
    let mut current_str = String::with_capacity(256);
    let mut current = !0;

    loop {
        if requested_update {
            requested_str.clear();
            requested_file.seek(SeekFrom::Start(0))
                .expect("failed to seek requested brightness");
            requested_file.read_to_string(&mut requested_str)
                .expect("failed to read requested brightness");
            requested = requested_str.trim().parse::<u64>()
                .expect("failed to parse requested brightness");
            requested_update = false;
            debug!("requested {}", requested);
        }

        if max_update {
            max_str.clear();
            max_file.seek(SeekFrom::Start(0))
                .expect("failed to seek max brightness");
            max_file.read_to_string(&mut max_str)
                .expect("failed to read max brightness");
            max = max_str.trim().parse::<u64>()
                .expect("failed to parse max brightness");
            max_update = false;
            debug!("max {}", requested);
        }

        let next = requested * 100 / max;
        while current != next {
            current = next;
            /* Smooth transition (may require use of xlib for performance)
            if current == !0 {
                current = next;
            } else if current < next {
                current += 1;
            } else if current > next {
                current -= 1;
            }
            */

            debug!("current {}%", current);

            current_str.clear();
            write!(current_str, "{}", current as f64 / 100.0)
                .expect("failed to serialize current brightness");

            process::Command::new("xrandr")
                .arg("--output").arg(output)
                .arg("--brightness").arg(&current_str)
                .status()
                .expect("failed to run xrandr");
        }

        // Use poll to establish a timeout
        let mut pollfd = libc::pollfd {
            fd: inotify.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = 1000;
        trace!(
            "poll fd: {}, events: {}, revents: {}, timeout: {})",
            pollfd.fd,
            pollfd.events,
            pollfd.revents,
            timeout
        );
        let count = unsafe { libc::poll(&mut pollfd, 1, timeout) };
        trace!(
            "poll fd: {}, events: {}, revents: {}, timeout: {} = {}",
            pollfd.fd,
            pollfd.events,
            pollfd.revents,
            timeout,
            count
        );
        if count < 0 {
            panic!("failed to poll: {}", Error::last_os_error());
        } else if count == 0 {
            // Timed out, update everything
            requested_update = true;
            max_update = true;
        } else {
            let mut buffer = [0; 1024];
            let events = inotify.read_events(&mut buffer)
                .expect("failed to read events");

            for event in events {
                trace!("event {:?}", event);
                if event.wd == requested_watch {
                    requested_update = true;
                }
                if event.wd == max_watch {
                    max_update = true;
                }
            }
        }
    }
}
