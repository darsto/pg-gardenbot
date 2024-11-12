use std::convert::TryFrom;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use levenshtein::levenshtein;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use winapi::um::winuser::{INPUT_u, SendInput, INPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP};

#[derive(Debug, Clone, Copy, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum CurrentlySelected {
    None,
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

        if levenshtein(item_prefix, "Thirst") < 2 {
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
    pub num_rounds: AtomicUsize,
    pub num_objects: AtomicUsize,
    pub extra_delay_secs: AtomicUsize,
    pub cur_selected: AtomicU8,
}

impl Grower {
    pub fn new() -> Arc<Self> {
        let (tx, rx): (Sender<GrowerThreadKick>, Receiver<GrowerThreadKick>) = mpsc::channel();

        let grower = Arc::new(Self {
            thread: Mutex::new(None),
            tx: Mutex::new(Some(tx)),
            num_rounds: AtomicUsize::new(0),
            num_objects: AtomicUsize::new(0),
            extra_delay_secs: AtomicUsize::new(0),
            cur_selected: AtomicU8::new(CurrentlySelected::None.into()),
        });

        let thread = GrowerThread::spawn(grower.clone(), rx);
        *grower.thread.lock().unwrap() = Some(thread);
        grower
    }

    pub fn start(&self) {
        // first break any currently running session
        self.num_rounds.store(0, Ordering::Relaxed);
        // then start a new one
        self.tx
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(GrowerThreadKick)
            .unwrap();
    }

    pub fn stop(&self) {
        self.num_rounds.store(0, Ordering::Relaxed);
    }

    pub fn join(&self) -> std::thread::Result<()> {
        self.stop();
        let handle = self.thread.lock().unwrap().take();
        let tx = self.tx.lock().unwrap().take();
        if let (Some(handle), Some(tx)) = (handle, tx) {
            drop(tx); // the thread should wake and exit
            handle.join()
        } else {
            std::thread::Result::Err(Box::new(()))
        }
    }
}

#[derive(Debug)]
struct GrowerThread {
    grower: Arc<Grower>,
    rx: Receiver<GrowerThreadKick>,
}

type GrowerThreadHandle = JoinHandle<()>;

/// Message sent to the GrowingThread
struct GrowerThreadKick;

impl GrowerThread {
    fn spawn(
        grower: Arc<Grower>,
        rx: Receiver<GrowerThreadKick>,
    ) -> GrowerThreadHandle {
        thread::spawn(move || {
            let inner = GrowerThread {
                grower,
                rx
            };
            inner.run();
        })
    }

    fn run(self) {
        loop {
            match self.rx.recv() {
                Err(_) => break,
                Ok(_) => {
                    while self.grower.num_rounds.load(Ordering::Relaxed) > 0 {
                        let _ = self.do_round();
                    }
                }
            }
        }
    }

    fn do_round(&self) -> Result<(), ()> {
        let grower = &self.grower;
        if grower.num_objects.load(Ordering::Relaxed) == 0 {
            return Ok(());
        }

        use CurrentlySelected as C;
        let sel: C = grower.cur_selected.load(Ordering::Relaxed).try_into().unwrap();
        match sel {
            C::None => {
                return self.interruptible_sleep(Duration::from_millis(5000));
            }
            C::Thisty => {
                self.do_use_round()?;
            }
            C::Hungry => {
                self.do_use_round()?;
            }
            C::Ripe => {
                self.do_use_round()?;

                assert!(self.grower.num_rounds.load(Ordering::Relaxed) > 0);
                self.grower.num_rounds.fetch_sub(1, Ordering::Relaxed);
            }
        }

        if self.grower.num_rounds.load(Ordering::Relaxed) == 0 {
            return Ok(());
        }

        for i in 0..5 {
            send_keypress(0x31 + i); // 1 key (or 2, 3, 4)
            self.interruptible_sleep(std::time::Duration::from_millis(225))?;
            send_keypress(0x31 + i); // 1 key (or 2, 3, 4)
            self.interruptible_sleep(std::time::Duration::from_millis(225))?;
            send_keypress(0x31 + i); // 1 key (or 2, 3, 4)
            self.interruptible_sleep(std::time::Duration::from_millis(225))?;
        }

        self.interruptible_sleep(std::time::Duration::from_millis(100))?;
        send_keypress(0x59); // Y key (next)
        Ok(())
    }

    fn do_use_round(&self) -> Result<(), ()> {
        let extra_delay_secs = self.grower.extra_delay_secs.load(Ordering::Relaxed);
        if extra_delay_secs > 0 {
            self.interruptible_sleep(Duration::from_secs(extra_delay_secs as u64))?;
        }

        for _ in 0..self.grower.num_objects.load(Ordering::Relaxed) {
            send_keypress(0x55); // U key (use)
            self.interruptible_sleep(std::time::Duration::from_millis(150))?;
            send_keypress(0x55); // U key (use)
            self.interruptible_sleep(std::time::Duration::from_millis(150))?;
            send_keypress(0x59); // Y key (next)
            self.interruptible_sleep(std::time::Duration::from_millis(150))?;
        }

        Ok(())
    }

    fn interruptible_sleep(&self, dur: Duration) -> std::result::Result<(), ()> {
        const SLEEP_TICK_MS: u128 = 100;
        if dur.as_millis() < SLEEP_TICK_MS {
            std::thread::sleep(dur);
            return match self.grower.num_rounds.load(Ordering::Relaxed) {
                0 => Err(()),
                _ => Ok(()),
            };
        }

        for _ in 0..(dur.as_millis() / SLEEP_TICK_MS) {
            std::thread::sleep(std::time::Duration::from_millis(SLEEP_TICK_MS as u64));
            if self.grower.num_rounds.load(Ordering::Relaxed) == 0 {
                return Err(());
            }
        }

        Ok(())
    }
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
