use env_logger::Env;
use inotify::{
    Inotify,
    WatchMask,
};
use log::{debug, error, info, trace};
use std::{fs, mem, process, ptr, slice, str};
use std::io::{Error, Read, Seek, SeekFrom};
use std::os::unix::io::{AsRawFd, RawFd};
use std::ptr::NonNull;
use x11::{xlib, xrandr};

pub struct ScreenNumber(libc::c_int);

pub struct RootWindow(libc::c_ulong);

pub struct Crtc(xrandr::RRCrtc);

pub struct CrtcGamma(NonNull<xrandr::XRRCrtcGamma>);

impl CrtcGamma {
    pub fn size(&self) -> libc::c_int {
        unsafe {
            self.0.as_ref().size
        }
    }

    pub fn channels(&mut self) -> (&mut [libc::c_ushort], &mut [libc::c_ushort], &mut [libc::c_ushort]) {
        unsafe {
            (
                slice::from_raw_parts_mut(
                    self.0.as_ref().red,
                    self.0.as_ref().size as usize
                ),
                slice::from_raw_parts_mut(
                    self.0.as_ref().green,
                    self.0.as_ref().size as usize
                ),
                slice::from_raw_parts_mut(
                    self.0.as_ref().blue,
                    self.0.as_ref().size as usize
                ),
            )
        }
    }
}

impl Drop for CrtcGamma {
    fn drop(&mut self) {
        unsafe {
            xrandr::XRRFreeGamma(self.0.as_ptr());
        }
    }
}

pub struct OutputInfo(NonNull<xrandr::XRROutputInfo>);

impl OutputInfo {
    pub fn name(&self) -> &[u8] {
        unsafe {
            slice::from_raw_parts(
                self.0.as_ref().name as *const u8,
                self.0.as_ref().nameLen as usize
            )
        }
    }

    pub fn crtc(&self) -> Option<Crtc> {
        let crtc = unsafe {
            self.0.as_ref().crtc
        };
        if crtc == 0 {
            None
        } else {
            Some(Crtc(crtc))
        }
    }
}

impl Drop for OutputInfo {
    fn drop(&mut self) {
        unsafe {
            xrandr::XRRFreeOutputInfo(self.0.as_ptr());
        }
    }
}

pub struct Output(xrandr::RROutput);

pub struct OutputsIter<'a> {
    items: &'a [xrandr::RROutput],
    i: usize,
}

impl<'a> Iterator for OutputsIter<'a> {
    type Item = Output;
    fn next(&mut self) -> Option<Output> {
        if let Some(item) = self.items.get(self.i) {
            self.i += 1;
            Some(Output(*item))
        } else {
            None
        }
    }
}

pub struct ScreenResources(NonNull<xrandr::XRRScreenResources>);

impl ScreenResources {
    pub fn outputs(&self) -> OutputsIter {
        let items = unsafe {
            slice::from_raw_parts(
                self.0.as_ref().outputs,
                self.0.as_ref().noutput as usize
            )
        };
        OutputsIter {
            items,
            i: 0,
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

    pub fn root_window(&self, screen_number: &ScreenNumber) -> RootWindow {
        RootWindow(unsafe {
            xlib::XRootWindow(self.0.as_ptr(), screen_number.0)
        })
    }

    pub fn get_screen_resources(&self, root_window: &RootWindow, current: bool) -> Option<ScreenResources> {
        NonNull::new(unsafe {
            if current {
                xrandr::XRRGetScreenResourcesCurrent(self.0.as_ptr(), root_window.0)
            } else {
                xrandr::XRRGetScreenResources(self.0.as_ptr(), root_window.0)
            }
        }).map(ScreenResources)
    }

    pub fn get_output_info(&self, resources: &ScreenResources, output: &Output) -> Option<OutputInfo> {
        NonNull::new(unsafe {
            xrandr::XRRGetOutputInfo(self.0.as_ptr(), resources.0.as_ptr(), output.0)
        }).map(OutputInfo)
    }

    pub fn get_crtc_gamma(&self, crtc: &Crtc) -> Option<CrtcGamma> {
        NonNull::new(unsafe {
            xrandr::XRRGetCrtcGamma(self.0.as_ptr(), crtc.0)
        }).map(CrtcGamma)
    }

    pub fn set_crtc_gamma(&mut self, crtc: &Crtc, gamma: &CrtcGamma) {
        unsafe {
            xrandr::XRRSetCrtcGamma(self.0.as_ptr(), crtc.0, gamma.0.as_ptr());
        }
    }

    pub fn select_input(&mut self, root_window: &RootWindow, mask: libc::c_int) {
        unsafe {
            xrandr::XRRSelectInput(self.0.as_ptr(), root_window.0, mask);
        }
    }

    pub fn flush(&mut self) {
        unsafe {
            xlib::XFlush(self.0.as_ptr());
        }
    }

    pub fn pending(&self) -> libc::c_int {
        unsafe {
            xlib::XPending(self.0.as_ptr())
        }
    }
}

impl AsRawFd for Display {
    fn as_raw_fd(&self) -> RawFd {
        unsafe {
            xlib::XConnectionNumber(self.0.as_ptr())
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        unsafe {
            xlib::XCloseDisplay(self.0.as_ptr());
        }
    }
}

fn xrandr_output_brightness(display: &mut Display, root_window: &RootWindow, output_name: &str, brightness_opt: Option<f64>) {
    if let Some(resources) = display.get_screen_resources(&root_window, true) {
        trace!("resources {:p}", resources.0.as_ptr());
        for output in resources.outputs() {
            trace!("output {:#x}", output.0);
            if let Some(info) = display.get_output_info(&resources, &output) {
                trace!("info {:p}", info.0.as_ptr());
                if let Ok(name) = str::from_utf8(info.name()) {
                    trace!("name {}", name);
                    if name.starts_with(output_name) {
                        trace!("matches {}", output_name);
                        if let Some(crtc) = info.crtc() {
                            trace!("crtc {:#x}", crtc.0);
                            if let Some(mut gamma) = display.get_crtc_gamma(&crtc) {
                                trace!("gamma {:p}", gamma.0.as_ptr());

                                let size = gamma.size() as usize;
                                let (red, green, blue) = gamma.channels();
                                for i in 0..size {
                                    let r = &mut red[i];
                                    let g = &mut green[i];
                                    let b = &mut blue[i];

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
                                display.set_crtc_gamma(&crtc, &gamma);

                                trace!("flush");
                                display.flush();
                            } else {
                                error!("failed to get X gamma info");
                            }
                        }
                    }
                }
            } else {
                error!("failed to get X output info");
            }
        }
    } else {
        error!("failed to get X screen resources");
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

    let mut display = Display::new().expect("failed to open X display");
    trace!("display {:p}", display.0.as_ptr());

    let mut xrr_event_base = 0;
    let mut xrr_error_base = 0;
    if unsafe { xrandr::XRRQueryExtension(display.0.as_ptr(), &mut xrr_event_base, &mut xrr_error_base) } == 0 {
        panic!("Xrandr extension not found");
    }
    trace!("xrr_event_base {:#x}, xrr_error_base {:#x}", xrr_event_base, xrr_error_base);

    let screen_number = display.default_screen_number();
    trace!("screen_number {:#x}", screen_number.0);

    let root_window = display.root_window(&screen_number);
    trace!("root_window {:#x}", root_window.0);

    display.select_input(&root_window, xrandr::RROutputChangeNotifyMask);

    let dbus_system = dbus::Connection::get_private(dbus::BusType::System)
        .expect("failed to connect to D-Bus system bus");
    dbus_system.add_match("interface='org.freedesktop.ColorManager',member='Changed'")
        .expect("failed to watch D-Bus signal");
    dbus_system.add_match("interface='org.freedesktop.ColorManager',member='DeviceChanged'")
        .expect("failed to watch D-Bus signal");
    dbus_system.add_match("interface='org.freedesktop.ColorManager',member='ProfileChanged'")
        .expect("failed to watch D-Bus signal");

    let dbus_session = dbus::Connection::get_private(dbus::BusType::Session)
        .expect("failed to connect to D-Bus session bus");
    dbus_session.add_match("interface='org.gnome.Mutter.DisplayConfig',member='MonitorsChanged'")
        .expect("failed to watch D-Bus signal");

    let mut pollfds = vec![libc::pollfd {
        fd: inotify.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }, libc::pollfd {
        fd: display.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }];

    let dbus_system_pollfd = pollfds.len();
    for watch in dbus_system.watch_fds() {
        pollfds.push(watch.to_pollfd());
    }

    let dbus_session_pollfd = pollfds.len();
    for watch in dbus_session.watch_fds() {
        pollfds.push(watch.to_pollfd());
    }

    let mut requested_update = true;
    let mut requested_str = String::with_capacity(256);
    let mut requested = 0;
    let mut max_update = true;
    let mut max_str = String::with_capacity(256);
    let mut max = 0;
    let mut current = !0;

    let mut timeout = -1;
    let mut timeout_times = 0;
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

            xrandr_output_brightness(&mut display, &root_window, output, if current == 100 {
                None
            } else {
                Some(current as f64 / 100.0)
            });

            debug!("current {}%", current);
        }

        // Use poll to establish a timeout
        for pollfd in pollfds.iter_mut() {
            pollfd.revents = 0;
        }
        trace!("poll fds: {}, timeout: {})", pollfds.len(), timeout);
        let count = unsafe {
            libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, timeout)
        };
        trace!("poll fds: {} timeout: {} = {}", pollfds.len(), timeout, count);

        if count < 0 {
            panic!("failed to poll: {}", Error::last_os_error());
        } else if count == 0 {
            // Update from timeout
            current = !0;
            if timeout_times == 0 {
                timeout = -1;
            } else {
                timeout_times -= 1;
            }
        } else {
            if pollfds[0].revents > 0 {
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

            if pollfds[1].revents > 0 {
                while display.pending() > 0 {
                    unsafe {
                        let mut event = mem::zeroed::<xlib::XEvent>();
                        xlib::XNextEvent(display.0.as_ptr(), &mut event);
                        trace!("event {:#x}", event.type_);
                        if event.type_ >= xrr_event_base {
                            let xrr_event_type = event.type_ - xrr_event_base;
                            trace!("xrr_event {:#x}", xrr_event_type);
                            if xrr_event_type == xrandr::RRNotify {
                                let notify_event: &xrandr::XRRNotifyEvent = event.as_ref();
                                trace!("notify_event {:?}", notify_event);
                                if notify_event.subtype == xrandr::RRNotify_OutputChange {
                                    let output_change_event: &xrandr::XRROutputChangeNotifyEvent = event.as_ref();
                                    trace!("output_change_event {:?}", output_change_event);
                                }

                                current = !0;
                            }
                        }
                    }
                }
            }

            for pollfd in pollfds[dbus_system_pollfd..dbus_session_pollfd].iter() {
                if pollfd.revents > 0 {
                    for item in dbus_system.watch_handle(pollfd.fd, dbus::WatchEvent::from_revents(pollfd.revents)) {
                        trace!("dbus system item {:?}", item);

                        // Mutter displays have changed, force a brightness update. A timeout is
                        // used because the gamma changes shortly after receiving this signal
                        // TODO: Figure out how to avoid mutter setting gamma
                        current = !0;
                        timeout = 100;
                        timeout_times = 10;
                    }
                }
            }

            for pollfd in pollfds[dbus_session_pollfd..].iter() {
                if pollfd.revents > 0 {
                    for item in dbus_session.watch_handle(pollfd.fd, dbus::WatchEvent::from_revents(pollfd.revents)) {
                        trace!("dbus session item {:?}", item);

                        // Mutter displays have changed, force a brightness update. A timeout is
                        // used because the gamma changes shortly after receiving this signal
                        // TODO: Figure out how to avoid mutter setting gamma
                        current = !0;
                        timeout = 100;
                        timeout_times = 10;
                    }
                }
            }
        }
    }
}
