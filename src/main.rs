#![windows_subsystem = "windows"]

extern crate levenshtein;
extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

use bmp::{BMPHeader, InfoHeader};
use grower::{CurrentlySelected, Grower};
use nwd::NwgUi;
use nwg::NativeUi;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::winuser::{LoadIconW, SetClassLongPtrA, GCLP_HICON, GCLP_HICONSM, MAKEINTRESOURCEW, WS_EX_TRANSPARENT};

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;
use std::{convert::TryFrom, sync::Arc};
use winapi::um::winbase::CREATE_NO_WINDOW;

mod bmp;
mod cropper;
mod grower;
mod richbuilder;
mod screenshot;

use cropper::Cropper;
use screenshot::{Rectangle, Screenshot};

#[derive(Default, NwgUi)]
pub struct BasicApp {
    #[nwg_control(size: (275, 225), position: (100, 100), icon: None, topmost: true, title: "Gardenbot", flags: "WINDOW|VISIBLE")]
    #[nwg_events(OnInit: [BasicApp::on_init], OnWindowClose: [BasicApp::on_close])]
    window: nwg::Window,

    #[nwg_control(flags: "VISIBLE|MULTI_LINE|DISABLED", position: (10, 10), size: (255, 65), ex_flags: WS_EX_TRANSPARENT)]
    rich_text_box: nwg::RichLabel,

    #[nwg_control(text: "Start", position: (175, 10), size: (90, 25))]
    #[nwg_events(OnButtonClick: [BasicApp::on_startstop_btn])]
    startstop_btn: nwg::Button,

    #[nwg_control(position: (10, 76), size: (255, 30))]
    select_area_bgimg: nwg::ImageFrame,

    #[nwg_control(position: (10, 76), size: (0, 0))]
    select_area_img: nwg::ImageFrame,

    #[nwg_control(text: "Selected Entity Area:", position: (10, 118), size: (150, 20))]
    select_area_label: nwg::Label,

    #[nwg_control(text: "Set", position: (175, 113), size: (90, 25))]
    #[nwg_events(OnButtonClick: [BasicApp::on_select_area_btn])]
    select_area_btn: nwg::Button,

    #[nwg_control(text: "Num rounds:", position: (10, 145), size: (150, 20))]
    num_rounds_label: nwg::Label,

    #[nwg_control(text: "0", flags: "NUMBER|VISIBLE", limit: 3, align: nwg::HTextAlign::Center, position: (176, 143), size: (88, 20))]
    num_rounds_input: nwg::TextInput,

    #[nwg_control(text: "Num growing objects:", position: (10, 170), size: (150, 20))]
    num_growing_objects_label: nwg::Label,

    #[nwg_control(text: "0", flags: "NUMBER|VISIBLE", limit: 2, align: nwg::HTextAlign::Center, position: (176, 168), size: (88, 20))]
    num_growing_objects_input: nwg::TextInput,

    #[nwg_control(text: "Extra delay (sec):", position: (10, 196), size: (150, 20))]
    extra_delay_sec_label: nwg::Label,

    #[nwg_control(text: "0", flags: "NUMBER|VISIBLE", limit: 2, align: nwg::HTextAlign::Center, position: (176, 193), size: (88, 20))]
    extra_delay_sec_input: nwg::TextInput,

    #[nwg_control(parent: window, interval: Duration::from_millis(1500))]
    #[nwg_events(OnTimerTick: [BasicApp::on_tick_1s])]
    timer_1s: nwg::AnimationTimer,

    state: Mutex<AppState>,
    // outside the state mutex because accessing it can block
    cropper: Mutex<Cropper>,
}

struct AppState {
    select_rect: Option<Rectangle<f64>>,
    select_rect_bitmap: nwg::Bitmap,
    scanned_str: Option<String>,

    org_num_rounds: usize,
    grower: Arc<Grower>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            select_rect: None,
            select_rect_bitmap: Default::default(),
            scanned_str: None,

            org_num_rounds: 0,
            grower: Grower::new(),
        }
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppState")
    }
}

impl BasicApp {
    fn on_init(&self) {
        self.timer_1s.start();
        let mut state = self.state.lock().unwrap();
        self.init_select_area_bgimg(&mut state);
        self.update_rich_text(&state);
    }

    fn on_select_area_btn(&self) {
        let screenshot = Screenshot::take();
        match self.cropper.lock().unwrap().apply(&screenshot) {
            Ok(Some(rect)) if rect.w > 0.0 && rect.h > 0.0 => {
                let mut state = self.state.lock().unwrap();
                state.select_rect = Some(rect);
            }
            Err(e) => {
                nwg::modal_info_message(&self.window, "Error", &format!("{:?}", e));
            }
            _ => {
                return;
            }
        }
        let mut state = self.state.lock().unwrap();
        self.refresh_logic_and_ui(&mut state);
    }

    fn on_startstop_btn(&self) {
        let mut state = self.state.lock().unwrap();

        if state.select_rect.is_none() {
            nwg::modal_info_message(&self.window, "Error", "You need to select the area first");
            return;
        }

        let num_rounds = self.num_rounds_input.text().parse::<usize>().unwrap();
        if num_rounds == 0 {
            nwg::modal_info_message(
                &self.window,
                "Error",
                "You need to set the number of rounds",
            );
            return;
        }

        self.startstop(&mut state, self.startstop_btn.text() == "Start");
        self.refresh_logic_and_ui(&mut state);
    }

    fn on_tick_1s(&self) {
        let mut state = self.state.lock().unwrap();
        self.refresh_logic_and_ui(&mut state);
    }

    fn init_select_area_bgimg(&self, state: &mut AppState) {
        let dimensions = self.select_area_bgimg.size();

        let mut bmpdata: Vec<u8> = Vec::new();
        let row_size = (dimensions.0 * 3).next_multiple_of(4) as usize;

        bmpdata
            .write_all(bytemuck::bytes_of(&BMPHeader::new(
                row_size * dimensions.1 as usize,
            )))
            .unwrap();
        bmpdata
            .write_all(bytemuck::bytes_of(&InfoHeader::new(
                dimensions.0,
                dimensions.1.wrapping_neg(),
            )))
            .unwrap();
        // we want the background black, so fill the rest of vec with zeroes
        bmpdata.resize(bmpdata.len() + row_size * dimensions.1 as usize, 0);

        let bitmap = &mut state.select_rect_bitmap;
        nwg::Bitmap::builder()
            .source_bin(Some(&bmpdata))
            .build(bitmap)
            .unwrap();

        self.select_area_bgimg.set_bitmap(Some(bitmap));
    }

    fn update_rich_text(&self, state: &AppState) {
        let mut rbuilder = richbuilder::RichBuilder::new(&self.rich_text_box);

        rbuilder.append("Gardenbot is ", nwg::CharFormat::default());
        if state.grower.running.load(Ordering::Relaxed) {
            rbuilder.append(
                "online",
                nwg::CharFormat {
                    effects: Some(nwg::CharEffects::BOLD),
                    text_color: Some([0, 140, 0]),
                    ..Default::default()
                },
            );
        } else {
            rbuilder.append(
                "offline",
                nwg::CharFormat {
                    effects: Some(nwg::CharEffects::BOLD),
                    text_color: Some([200, 0, 0]),
                    ..Default::default()
                },
            );
        }
        rbuilder.append("\n", nwg::CharFormat::default());
        if let Some(scanned_str) = &state.scanned_str {
            let status_strs = state.grower.status_str.lock().unwrap();
            rbuilder.append(status_strs[0].as_str(), nwg::CharFormat::default());
            rbuilder.append("\n", nwg::CharFormat::default());
            rbuilder.append(status_strs[1].as_str(), nwg::CharFormat::default());
            rbuilder.append("\n", nwg::CharFormat::default());

            rbuilder.append("Scanned: ", nwg::CharFormat::default());
            if scanned_str.trim().is_empty() {
                rbuilder.append("<nothing>", nwg::CharFormat::default());
            } else {
                rbuilder.append(scanned_str, nwg::CharFormat::default());
                rbuilder.append(" -> ", nwg::CharFormat::default());

                use CurrentlySelected as C;
                let parsed =
                    C::try_from(state.grower.cur_selected.load(Ordering::Relaxed)).unwrap();
                match parsed {
                    C::None => rbuilder.append("None", nwg::CharFormat::default()),
                    C::Growing => rbuilder.append("Growing", nwg::CharFormat::default()),
                    C::Thisty => rbuilder.append(
                        "Thirsty",
                        nwg::CharFormat {
                            effects: Some(nwg::CharEffects::BOLD),
                            text_color: Some([40, 130, 170]),
                            ..Default::default()
                        },
                    ),
                    C::Hungry => rbuilder.append(
                        "Hungry",
                        nwg::CharFormat {
                            effects: Some(nwg::CharEffects::BOLD),
                            text_color: Some([126, 98, 86]),
                            ..Default::default()
                        },
                    ),
                    C::Ripe => rbuilder.append(
                        "Ripe",
                        nwg::CharFormat {
                            effects: Some(nwg::CharEffects::BOLD),
                            text_color: Some([0, 140, 0]),
                            ..Default::default()
                        },
                    ),
                }
            }
        } else {
            rbuilder.append(
                "No entity area selected\n",
                nwg::CharFormat {
                    effects: Some(nwg::CharEffects::BOLD),
                    text_color: Some([100, 100, 100]),
                    ..Default::default()
                },
            );
        }
    }

    fn refresh_logic_and_ui(&self, state: &mut AppState) {
        self.refresh_logic(state);
        self.update_rich_text(state);
    }

    fn refresh_logic(&self, state: &mut AppState) {
        let running = state.grower.running.load(Ordering::Relaxed);
        if running != (self.startstop_btn.text() == "Stop") {
            self.startstop(state, running);
        }

        if let Some(bmpdata) = self.refresh_select_rect(state) {
            let scanned_str = state
                .scanned_str
                .insert(ocr_bmpdata(bmpdata.as_slice()).replace(['\n', '\r'], ""));

            use CurrentlySelected as C;
            let matching_selection = C::try_from(scanned_str.as_str()).unwrap();

            let prev_selection = C::try_from(
                state
                    .grower
                    .cur_selected
                    .swap(matching_selection.into(), Ordering::Relaxed),
            )
            .unwrap();

            if matching_selection != prev_selection
                && (prev_selection == C::None || prev_selection == C::Growing)
            {
                state.grower.kick();
            }
        } else {
            state.scanned_str = None;
        };
    }

    fn refresh_select_rect(&self, state: &mut AppState) -> Option<Vec<u8>> {
        let max_dimensions = self.select_area_bgimg.size();
        let mut rect = state.select_rect?;
        rect.w = f64::min(rect.w, max_dimensions.0 as f64);
        rect.h = f64::min(rect.h, max_dimensions.1 as f64);

        let screenshot = Screenshot::take();
        let bmpdata = screenshot.get_bmp_data(Rectangle {
            x: rect.x as u32,
            y: rect.y as u32,
            w: rect.w as u32,
            h: rect.h as u32,
        });

        let bitmap = &mut state.select_rect_bitmap;
        nwg::Bitmap::builder()
            .source_bin(Some(&bmpdata))
            .build(bitmap)
            .unwrap();

        self.select_area_img.set_bitmap(Some(bitmap));
        self.select_area_img.set_size(rect.w as u32, rect.h as u32);
        self.select_area_img.set_visible(true);

        Some(bmpdata)
    }

    fn startstop(&self, state: &mut AppState, start: bool) {
        if start {
            state.org_num_rounds = self.num_rounds_input.text().parse::<usize>().unwrap();

            let grower = &state.grower;
            grower
                .num_rounds
                .store(state.org_num_rounds, Ordering::Relaxed);
            grower.num_objects.store(
                self.num_growing_objects_input
                    .text()
                    .parse::<usize>()
                    .unwrap(),
                Ordering::Relaxed,
            );
            grower.extra_delay_secs.store(
                self.extra_delay_sec_input.text().parse::<usize>().unwrap(),
                Ordering::Relaxed,
            );
            grower.start();

            self.startstop_btn.set_text("Stop");
            self.num_rounds_input.set_readonly(true);
            self.num_rounds_input.set_enabled(false);
            self.num_growing_objects_input.set_readonly(true);
            self.num_growing_objects_input.set_enabled(false);
            self.extra_delay_sec_input.set_readonly(true);
            self.extra_delay_sec_input.set_enabled(false);
            self.select_area_btn.set_enabled(false);
        } else {
            state.grower.stop();
            self.num_rounds_input
                .set_text(&state.org_num_rounds.to_string());

            self.startstop_btn.set_text("Start");
            self.num_rounds_input.set_readonly(false);
            self.num_rounds_input.set_enabled(true);
            self.num_growing_objects_input.set_readonly(false);
            self.num_growing_objects_input.set_enabled(true);
            self.extra_delay_sec_input.set_readonly(false);
            self.extra_delay_sec_input.set_enabled(true);
            self.select_area_btn.set_enabled(true);
        }

        self.timer_1s.stop();
        self.timer_1s.start();
    }

    fn on_close(&self) {
        nwg::stop_thread_dispatch();
    }
}

fn ocr_bmpdata(bmpdata: &[u8]) -> String {
    use std::process::Command;
    let mut proc = Command::new("C:\\Program Files\\ImageMagick\\convert.exe")
        .args([
            "fd:0",
            "-color-threshold",
            "sRGB(70,70,70)-sRGB(230,210,160)",
            "-negate",
            "fd:1",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to execute process");

    let mut stdin = proc.stdin.take().expect("Failed to write to stdin");
    stdin.write_all(bmpdata).unwrap();
    drop(stdin);

    let output = proc.wait_with_output().expect("Failed to read stdout");
    let processed = output.stdout;

    if !output.status.success() {
        panic!(
            "imagemagick threshold filter (image enhancing) failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let mut proc = Command::new("C:\\Program Files\\Tesseract-OCR\\tesseract.exe")
        .args(["stdin", "stdout"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to execute process");

    let mut stdin = proc.stdin.take().expect("Failed to write to stdin");
    stdin.write_all(processed.as_slice()).unwrap();
    drop(stdin);

    let output = proc.wait_with_output().expect("Failed to read stdout");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn main() {
    // print messages in the parent console, if any
    unsafe { winapi::um::wincon::AttachConsole(u32::MAX) };

    // TODO: is this needed?
    unsafe {
        winapi::um::shellscalingapi::SetProcessDpiAwareness(
            winapi::um::shellscalingapi::PROCESS_DPI_UNAWARE,
        );
    }

    nwg::init().expect("Failed to init Native Windows GUI");

    let mut font = nwg::Font::default();
    nwg::Font::builder()
        .family("Segoe UI")
        .size(16)
        .build(&mut font)
        .expect("Failed to build font");

    nwg::Font::set_global_default(Some(font));

    let app = BasicApp::build_ui(Default::default()).expect("Failed to build UI");

    unsafe {
        let h_instance = GetModuleHandleW(std::ptr::null());
        assert!(!h_instance.is_null());
        let icon = LoadIconW(h_instance, MAKEINTRESOURCEW(32512));
        assert!(!icon.is_null());
        let nwg::ControlHandle::Hwnd(win_hwnd) = app.window.handle else {
            unreachable!();
        };
        // WM_SETICON doesn't update taskbar icon on Windows 11, so update the Window Class instead
        SetClassLongPtrA(win_hwnd, GCLP_HICON, icon as _);
        SetClassLongPtrA(win_hwnd, GCLP_HICONSM, icon as _);
    }

    nwg::dispatch_thread_events();

    println!("Exiting");
}
