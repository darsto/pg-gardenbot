use std::convert::TryFrom;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use levenshtein::levenshtein;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use winapi::shared::windef::HWND;
use winapi::um::winuser::{
    FindWindowA, GetForegroundWindow, INPUT_u, SendInput, SetForegroundWindow, INPUT,
    INPUT_KEYBOARD, KEYEVENTF_KEYUP,
};

#[derive(Debug, Clone, Copy, PartialEq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum CurrentlySelected {
    None,
    Growing,
    Thisty,
    Hungry,
    Ripe,
}

impl TryFrom<&str> for CurrentlySelected {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut parts = value.split_whitespace();
        let item_prefix = loop {
            let Some(word) = parts.next() else {
                return Ok(Self::None);
            };

            if word.len() >= 4 {
                break word;
            }
        };

        // if the space was not detected (e.g. parsed as .), then limit to 6 - we don't need more
        let item_prefix = &item_prefix[0..std::cmp::min(item_prefix.len(), 6)];

        if levenshtein(item_prefix, "Growin") < 2 {
            Ok(Self::Growing)
        } else if levenshtein(item_prefix, "Thirst") < 2 {
            Ok(Self::Thisty)
        } else if levenshtein(item_prefix, "Hungry") < 2 {
            Ok(Self::Hungry)
        } else if levenshtein(item_prefix, "Bloomi") < 2 || levenshtein(item_prefix, "Ripe") < 2 {
            Ok(Self::Ripe)
        } else {
            Ok(Self::None)
        }
    }
}

#[derive(Debug)]
pub struct Grower {
    thread: Mutex<Option<GrowerThreadHandle>>,
    tx: Mutex<Option<Sender<GrowerThreadKick>>>,
    pub running: AtomicBool,
    pub num_rounds: AtomicUsize,
    pub num_objects: AtomicUsize,
    pub extra_delay_secs: AtomicUsize,
    pub cur_selected: AtomicU8,
    pub status_str: Mutex<[String; 2]>,
}

impl Grower {
    pub fn new() -> Arc<Self> {
        let (tx, rx): (Sender<GrowerThreadKick>, Receiver<GrowerThreadKick>) = mpsc::channel();

        let grower = Arc::new(Self {
            thread: Mutex::new(None),
            tx: Mutex::new(Some(tx)),
            running: AtomicBool::new(false),
            num_rounds: AtomicUsize::new(0),
            num_objects: AtomicUsize::new(0),
            extra_delay_secs: AtomicUsize::new(0),
            cur_selected: AtomicU8::new(CurrentlySelected::None.into()),
            status_str: Mutex::new([String::with_capacity(256), String::with_capacity(256)]),
        });

        let thread = GrowerThread::spawn(grower.clone(), rx);
        *grower.thread.lock().unwrap() = Some(thread);
        grower
    }

    pub fn start(&self) {
        // first break any currently running session
        self.running.store(false, Ordering::Relaxed);
        // then start a new one
        self.kick();
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn kick(&self) {
        self.tx
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(GrowerThreadKick)
            .unwrap();
    }
}

#[derive(Debug)]
struct GrowerThread {
    grower: Arc<Grower>,
    rx: Receiver<GrowerThreadKick>,
    window: Option<HWND>,
    abort_on_missing_selection: bool,
}

type GrowerThreadHandle = JoinHandle<()>;

/// Message sent to the GrowingThread
struct GrowerThreadKick;

impl GrowerThread {
    fn spawn(grower: Arc<Grower>, rx: Receiver<GrowerThreadKick>) -> GrowerThreadHandle {
        thread::spawn(move || {
            let inner = GrowerThread {
                grower,
                rx,
                window: None,
                abort_on_missing_selection: false,
            };
            inner.run();
        })
    }

    fn run(mut self) {
        loop {
            match self.rx.recv() {
                Err(_) => break,
                Ok(_) => {
                    self.window = find_window("Project Gorgon");
                    if self.window.is_none() {
                        nwg::error_message("Error", "Can't find Project Gorgon window");
                        continue;
                    }
                    self.grower.running.store(true, Ordering::Relaxed);
                    while self.can_continue().is_ok() {
                        let _ = self.do_round();
                    }
                    self.grower.running.store(false, Ordering::Relaxed);
                }
            }
        }
    }

    fn do_round(&mut self) -> Result<(), ()> {
        let grower = &self.grower;
        if grower.num_objects.load(Ordering::Relaxed) == 0 {
            return Ok(());
        }

        use CurrentlySelected as C;
        let sel: C = grower
            .cur_selected
            .load(Ordering::Relaxed)
            .try_into()
            .unwrap();
        match sel {
            C::None | C::Growing => {
                self.update_status_str(Some("Waiting 5s ..."), Some(""));
                self.interruptible_sleep(Duration::from_millis(5000))?;
            }
            C::Thisty => {
                self.abort_on_missing_selection = true;
                self.update_status_str(Some("Watering!"), Some(""));
                self.do_use_round()?;
            }
            C::Hungry => {
                self.abort_on_missing_selection = true;
                self.update_status_str(Some("Fertilizing!"), Some(""));
                self.do_use_round()?;
            }
            C::Ripe => {
                self.abort_on_missing_selection = false;
                self.update_status_str(Some("Harvesting!"), Some(""));
                self.do_use_round()?;

                assert!(self.grower.num_rounds.load(Ordering::Relaxed) > 0);
                self.grower.num_rounds.fetch_sub(1, Ordering::Relaxed);
                self.can_continue()?;

                self.update_status_str(Some("Replanting!"), Some(""));
                let prev_hwnd = set_hwnd_focus(self.window.unwrap());
                for i in 0..5 {
                    send_keypress(0x31 + i); // 1 key (or 2, 3, 4)
                    self.interruptible_sleep(std::time::Duration::from_millis(225))?;
                    send_keypress(0x31 + i); // 1 key (or 2, 3, 4)
                    self.interruptible_sleep(std::time::Duration::from_millis(225))?;
                    send_keypress(0x31 + i); // 1 key (or 2, 3, 4)
                    self.interruptible_sleep(std::time::Duration::from_millis(225))?;
                }

                self.interruptible_sleep(std::time::Duration::from_millis(150))?;
                send_keypress(0x59); // Y key (next)
                self.interruptible_sleep(std::time::Duration::from_millis(150))?;
                set_hwnd_focus(prev_hwnd);
            }
        };

        Ok(())
    }

    fn do_use_round(&self) -> Result<(), ()> {
        let extra_delay_secs = self.grower.extra_delay_secs.load(Ordering::Relaxed);
        if extra_delay_secs > 0 {
            self.update_status_str(None, Some("Waiting extra seconds ..."));
            self.interruptible_sleep(Duration::from_secs(extra_delay_secs as u64))?;
            self.update_status_str(None, Some(""));
        }

        let prev_hwnd = set_hwnd_focus(self.window.unwrap());
        for _ in 0..self.grower.num_objects.load(Ordering::Relaxed) {
            send_keypress(0x55); // U key (use)
            self.interruptible_sleep(std::time::Duration::from_millis(150))?;
            send_keypress(0x55); // U key (use)
            self.interruptible_sleep(std::time::Duration::from_millis(150))?;
            send_keypress(0x59); // Y key (next)
            self.interruptible_sleep(std::time::Duration::from_millis(150))?;
        }
        set_hwnd_focus(prev_hwnd);

        Ok(())
    }

    fn can_continue(&self) -> std::result::Result<(), ()> {
        if !self.grower.running.load(Ordering::Relaxed) {
            return Err(());
        }

        if self.grower.num_rounds.load(Ordering::Relaxed) == 0 {
            self.update_status_str(Some("Finished all rounds"), Some(""));
            return Err(());
        }
        if self.abort_on_missing_selection
            && CurrentlySelected::try_from(self.grower.cur_selected.load(Ordering::Relaxed))
                .unwrap()
                == CurrentlySelected::None
        {
            self.update_status_str(Some(""), Some("Selection changed abruptly! Stopping"));
            return Err(());
        }
        Ok(())
    }

    fn interruptible_sleep(&self, dur: Duration) -> std::result::Result<(), ()> {
        const SLEEP_TICK_MS: u128 = 100;
        if dur.as_millis() < SLEEP_TICK_MS {
            std::thread::sleep(dur);
            return self.can_continue();
        }

        for _ in 0..(dur.as_millis() / SLEEP_TICK_MS) {
            std::thread::sleep(std::time::Duration::from_millis(SLEEP_TICK_MS as u64));
            self.can_continue()?;
        }

        Ok(())
    }

    fn update_status_str(&self, s0: Option<&str>, s1: Option<&str>) {
        let mut s = self.grower.status_str.lock().unwrap();
        if let Some(s0) = s0 {
            s[0].clear();
            s[0].push_str(s0);
        }
        if let Some(s1) = s1 {
            s[1].clear();
            s[1].push_str(s1);
        }
    }
}

fn find_window(name: &str) -> Option<HWND> {
    let c_name = CString::new(name).unwrap();

    let hwnd = unsafe { FindWindowA(std::ptr::null_mut(), c_name.as_ptr()) };
    if hwnd.is_null() {
        return None;
    }

    Some(hwnd)
}

fn send_keypress(key: u16) {
    let mut ip = INPUT {
        type_: INPUT_KEYBOARD,
        u: unsafe { MaybeUninit::<INPUT_u>::zeroed().assume_init() },
    };

    // press
    unsafe {
        ip.u.ki_mut().wVk = key; // virtual-key code
        SendInput(1, &mut ip, core::mem::size_of_val(&ip) as i32);
    }

    // release
    unsafe {
        ip.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
        SendInput(1, &mut ip, core::mem::size_of_val(&ip) as i32);
    };
}

fn set_hwnd_focus(hwnd: HWND) -> HWND {
    let prev = unsafe { GetForegroundWindow() };
    unsafe { SetForegroundWindow(hwnd) };
    std::thread::sleep(Duration::from_millis(100));
    prev
}
