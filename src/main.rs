use env_logger::Env;
use inotify::{
    Inotify,
    WatchMask,
};
use log::{debug, error, info, trace};
use std::{fs, process, ptr, slice, str};
use std::io::{Error, Read, Seek, SeekFrom};
use std::os::unix::io::AsRawFd;
use std::ptr::NonNull;
use x11::{xlib, xrandr};

pub struct ScreenNumber(libc::c_int);

pub struct RootWindow(libc::c_ulong);

pub struct ScreenResources(NonNull<xrandr::XRRScreenResources>);

impl ScreenResources {
    pub fn outputs(&self) -> &[xrandr::RROutput] {
        unsafe {
            slice::from_raw_parts(
                self.0.as_ref().outputs,
                self.0.as_ref().noutput as usize
            )
        }
    }
}

impl Drop for ScreenResources {
    fn drop(&mut self) {
        unsafe {
            xrandr::XRRFreeScreenResources(self.0.as_ptr());
        }
    }
}

pub struct Display(NonNull<xlib::Display>);

impl Display {
    pub fn new() -> Option<Self> {
        NonNull::new(unsafe {
            xlib::XOpenDisplay(ptr::null())
        }).map(Self)
    }

    pub fn default_screen_number(&self) -> ScreenNumber {
        ScreenNumber(unsafe {
            xlib::XDefaultScreen(self.0.as_ptr())
        })
    }

    pub fn root_window(&self, screen_number: ScreenNumber) -> RootWindow {
        RootWindow(unsafe {
            xlib::XRootWindow(self.0.as_ptr(), screen_number.0)
        })
    }

    pub fn get_screen_resources(&self, root_window: RootWindow, current: bool) -> Option<ScreenResources> {
        NonNull::new(unsafe {
            if current {
                xrandr::XRRGetScreenResourcesCurrent(self.0.as_ptr(), root_window.0)
            } else {
                xrandr::XRRGetScreenResources(self.0.as_ptr(), root_window.0)
            }
        }).map(ScreenResources)
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        unsafe {
            xlib::XCloseDisplay(self.0.as_ptr());
        }
    }
}

unsafe fn xrandr_output_brightness(output_name: &str, brightness_opt: Option<f64>) {
    let display = xlib::XOpenDisplay(ptr::null());
    if ! display.is_null() {
        trace!("display {:p}", display);

        let screen = xlib::XDefaultScreen(display);
        trace!("screen {:#x}", screen);

        let root = xlib::XRootWindow(display, screen);
        trace!("root {:#x}", root);

        //TODO: Check xrandr version

        let resources = xrandr::XRRGetScreenResourcesCurrent(display, root);
        if ! resources.is_null() {
            trace!("resources {:p}", resources);

            for output in slice::from_raw_parts_mut(
                (*resources).outputs,
                (*resources).noutput as usize
            ) {
                trace!("output {:#x}", output);

                let info = xrandr::XRRGetOutputInfo(display, resources, *output);
                if ! info.is_null() {
                    trace!("info {:p}", info);

                    let name_bytes = slice::from_raw_parts(
                        (*info).name as *const u8,
                        (*info).nameLen as usize
                    );
                    if let Ok(name) = str::from_utf8(name_bytes) {
                        trace!("name {}", name);

                        if (*info).crtc != 0 && name.starts_with(output_name) {
                            trace!("crtc {:#x}", (*info).crtc);

                            let gamma = xrandr::XRRGetCrtcGamma(display, (*info).crtc);
                            if ! gamma.is_null() {
                                trace!("gamma {:p}", gamma);

                                let size = (*gamma).size;

                                for i in 0..size as usize {
                                    let r = &mut *(*gamma).red.add(i);
                                    let g = &mut *(*gamma).green.add(i);
                                    let b = &mut *(*gamma).blue.add(i);

                                    let calulate_value = |gamma_opt: Option<f64>| -> u16 {
                                        // Calculate standard gamma value
                                        let mut value = (i as f64) / ((size - 1) as f64);

                                        // Apply gamma for channel
                                        if let Some(gamma) = gamma_opt {
                                            value = value.powf(1.0 / gamma);
                                        }

                                        // Apply brightness
                                        if let Some(brightness) = brightness_opt {
                                            value *= brightness;
                                        }

                                        // Convert to short
                                        (value.min(1.0) * 65535.0) as u16
                                    };

                                    *r = calulate_value(None);
                                    *g = calulate_value(None);
                                    *b = calulate_value(None);
                                }

                                trace!("set gamma");
                                xrandr::XRRSetCrtcGamma(display, (*info).crtc, gamma);

                                xrandr::XRRFreeGamma(gamma);
                            } else {
                                error!("failed to get X gamma info");
                            }
                        }
                    }

                    xrandr::XRRFreeOutputInfo(info);
                } else {
                    error!("failed to get X output info");
                }
            }

            xrandr::XRRFreeScreenResources(resources);
        } else {
            error!("failed to get X screen resources");
        }

        xlib::XCloseDisplay(display);
    } else {
        error!("failed to open X display");
    }
}

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
            debug!("max {}", max);
        }

        let next = requested * 100 / max;
        debug!("next {}%", next);
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


            unsafe {
                xrandr_output_brightness(output, if current == 100 {
                    None
                } else {
                    Some(current as f64 / 100.0)
                });
            }
            debug!("current {}%", current);
        }

        // Use poll to establish a timeout
        let mut pollfd = libc::pollfd {
            fd: inotify.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = 5000;
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
            current = !0;
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
