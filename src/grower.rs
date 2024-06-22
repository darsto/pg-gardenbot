use std::cell::RefCell;
use std::convert::TryFrom;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use levenshtein::levenshtein;
use winapi::um::winuser::{INPUT_u, SendInput, INPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP};

#[derive(Debug, Clone, Copy)]
pub enum CurrentlySelected {
    Thisty,
    Hungry,
    Ripe,
}

impl TryFrom<&str> for CurrentlySelected {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let Some(item_prefix) = value.split_whitespace().next() else {
            return Err(());
        };

        if item_prefix.len() < 4 {
            return Err(());
        }

        // if the space was not detected (e.g. parsed as .), then limit to 6 - we don't need more
        let item_prefix = &item_prefix[0..std::cmp::min(item_prefix.len(), 6)];

        if levenshtein(item_prefix, "Thirst") < 2 {
            Ok(Self::Thisty)
        } else if levenshtein(item_prefix, "Hungry") < 2 {
            Ok(Self::Hungry)
        } else if levenshtein(item_prefix, "Bloomi") < 2 || levenshtein(item_prefix, "Ripe") < 2 {
            Ok(Self::Ripe)
        } else {
            Err(())
        }
    }
}

#[derive(Debug)]
pub struct Grower {
    thread: RefCell<Option<GrowerThreadHandle>>,
    tx: Option<Sender<GrowerThreadKick>>,
    num_rounds: Arc<AtomicUsize>,
    cur_selected: Arc<Mutex<Option<CurrentlySelected>>>,
}

impl Grower {
    pub fn new() -> Self {
        let (tx, rx): (Sender<GrowerThreadKick>, Receiver<GrowerThreadKick>) = mpsc::channel();
        let num_rounds = Arc::new(AtomicUsize::new(0));
        let cur_selected = Arc::new(Mutex::new(None));
        let thread = GrowerThread::spawn(rx, num_rounds.clone(), cur_selected.clone());

        Self {
            thread: RefCell::new(Some(thread)),
            tx: Some(tx),
            num_rounds,
            cur_selected,
        }
    }

    pub fn start(&self, num_rounds: usize, num_objects: usize, use_delay: Duration) {
        // first break any currently running session
        self.num_rounds.store(0, Ordering::Release);
        // then start a new one
        self.tx
            .as_ref()
            .unwrap()
            .send(GrowerThreadKick {
                num_rounds,
                num_objects,
                use_delay,
            })
            .unwrap();
    }

    pub fn stop(&self) {
        self.num_rounds.store(0, Ordering::Release);
    }

    pub fn update_selected(&self, sel: Option<CurrentlySelected>) {
        *self.cur_selected.lock().unwrap() = sel;
    }

    pub fn remaining_rounds(&self) -> usize {
        self.num_rounds.load(Ordering::Acquire)
    }

    pub fn join(&mut self) -> std::thread::Result<()> {
        self.stop();
        let handle = self.thread.replace(None);
        let tx = self.tx.take();
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
    num_rounds: Arc<AtomicUsize>,
    rx: Receiver<GrowerThreadKick>,

    num_objects: usize,
    use_delay: Duration,
    cur_selected: Arc<Mutex<Option<CurrentlySelected>>>,
}

type GrowerThreadHandle = JoinHandle<()>;

/// Message sent to the GrowingThread
struct GrowerThreadKick {
    num_rounds: usize,
    num_objects: usize,
    use_delay: Duration,
}

impl GrowerThread {
    fn spawn(
        rx: Receiver<GrowerThreadKick>,
        num_rounds: Arc<AtomicUsize>,
        cur_selected: Arc<Mutex<Option<CurrentlySelected>>>,
    ) -> GrowerThreadHandle {
        thread::spawn(move || {
            let inner = GrowerThread {
                num_rounds,
                rx,

                num_objects: 0,
                use_delay: Duration::ZERO,
                cur_selected,
            };
            inner.run();
        })
    }

    fn run(mut self) {
        loop {
            match self.rx.recv() {
                Err(_) => break,
                Ok(kick) => {
                    self.num_objects = kick.num_objects;
                    self.use_delay = kick.use_delay;
                    // keep running until either we finish or get stopped
                    self.num_rounds.store(kick.num_rounds, Ordering::Release);
                    while self.num_rounds.load(Ordering::Acquire) > 0 {
                        let _ = self.do_round();
                    }
                }
            }
        }
    }

    fn do_round(&self) -> Result<(), ()> {
        if self.num_objects == 0 {
            return Ok(());
        }

        use CurrentlySelected as C;
        let sel = *self.cur_selected.lock().unwrap();
        match sel {
            None => {
                return self.interruptible_sleep(Duration::from_millis(5000));
            }
            Some(C::Thisty) => {
                self.do_use_round()?;
            }
            Some(C::Hungry) => {
                self.do_use_round()?;
            }
            Some(C::Ripe) => {
                self.do_use_round()?;

                assert!(self.num_rounds.load(Ordering::Acquire) > 0);
                self.num_rounds.fetch_sub(1, Ordering::AcqRel);
            }
        }

        if self.num_rounds.load(Ordering::Acquire) == 0 {
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
        if !self.use_delay.is_zero() {
            self.interruptible_sleep(self.use_delay)?;
        }

        for _ in 0..self.num_objects {
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
            return match self.num_rounds.load(Ordering::Acquire) {
                0 => Err(()),
                _ => Ok(()),
            };
        }

        for _ in 0..(dur.as_millis() / SLEEP_TICK_MS) {
            std::thread::sleep(std::time::Duration::from_millis(SLEEP_TICK_MS as u64));
            if self.num_rounds.load(Ordering::Acquire) == 0 {
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
