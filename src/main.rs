#![windows_subsystem = "windows"]

extern crate levenshtein;
extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

use bmp::{BMPHeader, InfoHeader};
use grower::Grower;
use nwd::NwgUi;
use nwg::NativeUi;

use std::convert::TryFrom;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
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
    #[nwg_control(size: (275, 290), position: (100, 100), icon: None, topmost: true, title: "Gardenbot", flags: "WINDOW|VISIBLE")]
    #[nwg_events(OnInit: [BasicApp::on_init], OnWindowClose: [BasicApp::on_close])]
    window: nwg::Window,

    #[nwg_control(flags: "VISIBLE|MULTI_LINE|DISABLED", position: (10, 10), size: (156, 65))]
    rich_text_box: nwg::RichLabel,

    #[nwg_control(text: "Start", position: (175, 10), size: (90, 25))]
    #[nwg_events(OnButtonClick: [BasicApp::on_startstop_btn])]
    startstop_btn: nwg::Button,

    #[nwg_control(text: "Num rounds:", position: (10, 79), size: (150, 20))]
    num_rounds_label: nwg::Label,

    #[nwg_control(text: "0", flags: "NUMBER|VISIBLE", limit: 3, align: nwg::HTextAlign::Center, position: (176, 77), size: (88, 20))]
    num_rounds_input: nwg::TextInput,

    #[nwg_control(text: "Num growing objects:", position: (10, 104), size: (150, 20))]
    num_growing_objects_label: nwg::Label,

    #[nwg_control(text: "0", flags: "NUMBER|VISIBLE", limit: 2, align: nwg::HTextAlign::Center, position: (176, 102), size: (88, 20))]
    num_growing_objects_input: nwg::TextInput,

    #[nwg_control(text: "Extra delay (sec):", position: (10, 129), size: (150, 20))]
    extra_delay_sec_label: nwg::Label,

    #[nwg_control(text: "0", flags: "NUMBER|VISIBLE", limit: 2, align: nwg::HTextAlign::Center, position: (176, 127), size: (88, 20))]
    extra_delay_sec_input: nwg::TextInput,

    #[nwg_control(text: "Selected Entity Area:", position: (10, 155), size: (150, 20))]
    select_area_label: nwg::Label,

    #[nwg_control(text: "Set", position: (175, 150), size: (90, 25))]
    #[nwg_events(OnButtonClick: [BasicApp::on_select_area_btn])]
    select_area_btn: nwg::Button,

    #[nwg_control(position: (10, 180), size: (255, 100))]
    select_area_bgimg: nwg::ImageFrame,

    #[nwg_control(position: (10, 180), size: (0, 0))]
    select_area_img: nwg::ImageFrame,

    #[nwg_control(parent: window, interval: Duration::from_millis(5000))]
    #[nwg_events(OnTimerTick: [BasicApp::on_tick_5s])]
    select_update_timer: nwg::AnimationTimer,

    state: Mutex<AppState>,
    // outside the state mutex because accessing it can block
    cropper: Mutex<CropperWrapper>,
}

struct AppState {
    select_rect: Option<Rectangle<f64>>,
    select_rect_bitmap: nwg::Bitmap,

    org_num_rounds: usize,
    grower: Grower,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            select_rect: None,
            select_rect_bitmap: Default::default(),

            org_num_rounds: 0,
            grower: Grower::new(),
        }
    }
}

struct CropperWrapper(Cropper);

impl Default for CropperWrapper {
    fn default() -> Self {
        Self(Cropper::new())
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppState")
    }
}

impl BasicApp {
    fn on_init(&self) {
        self.select_update_timer.start();
        let mut state = self.state.lock().unwrap();
        self.init_select_area_bgimg(&mut state);
        self.update_rich_text(&mut state, "");
    }

    fn init_select_area_bgimg(&self, state: &mut AppState) {
        let dimensions: (u32, u32) = (255, 100);

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

    fn update_rich_text(&self, state: &mut AppState, scanned_str: &str) {
        let mut rbuilder = richbuilder::RichBuilder::new(&self.rich_text_box);

        rbuilder.append("Gardenbot is ", nwg::CharFormat::default());
        if state.grower.remaining_rounds() > 0 {
            rbuilder.append(
                "online",
                nwg::CharFormat {
                    effects: Some(nwg::CharEffects::BOLD),
                    text_color: Some([0, 200, 0]),
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
        rbuilder.append("\r\n", nwg::CharFormat::default());
        rbuilder.append("Scanned:\r\n", nwg::CharFormat::default());
        rbuilder.append(scanned_str, nwg::CharFormat::default());
    }

    fn on_tick_5s(&self) {
        let mut state = self.state.lock().unwrap();
        self.update_logic(&mut state);
    }

    fn update_logic(&self, state: &mut AppState) {
        let running = state.grower.remaining_rounds() > 0;
        if running != (self.startstop_btn.text() == "Stop") {
            self.startstop(state, running);
            self.update_rich_text(state, "");
            return;
        }

        let Some(bmpdata) = self.update_select_rect(state) else {
            self.update_rich_text(state, "");
            return;
        };

        let scanned_str = ocr_bmpdata(bmpdata.as_slice()).replace('\n', "");
        self.update_rich_text(state, &scanned_str);

        let matching_selection = grower::CurrentlySelected::try_from(scanned_str.as_str()).ok();
        state.grower.update_selected(matching_selection);
    }

    fn update_select_rect(&self, state: &mut AppState) -> Option<Vec<u8>> {
        let Some(rect) = state.select_rect else {
            return None;
        };

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
        self.select_area_img
            .set_size(u32::min(rect.w as u32, 255), u32::min(rect.h as u32, 100));

        Some(bmpdata)
    }

    fn on_select_area_btn(&self) {
        let screenshot = Screenshot::take();
        match self.cropper.lock().unwrap().0.apply(&screenshot) {
            Ok(Some(rect)) => {
                let mut state = self.state.lock().unwrap();
                state.select_rect = Some(rect);
                self.update_select_rect(&mut state);
            }
            Ok(None) => {}
            Err(e) => {
                nwg::modal_info_message(&self.window, "Error", &format!("{:?}", e));
            }
        }
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
        self.update_logic(&mut state);
    }

    fn startstop(&self, state: &mut AppState, start: bool) {
        if start {
            state.org_num_rounds = self.num_rounds_input.text().parse::<usize>().unwrap();
            state.grower.start(
                state.org_num_rounds,
                self.num_growing_objects_input
                    .text()
                    .parse::<usize>()
                    .unwrap(),
                Duration::from_secs(self.extra_delay_sec_input.text().parse::<u64>().unwrap()),
            );

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
                .set_text(&format!("{}", state.org_num_rounds));

            self.startstop_btn.set_text("Start");
            self.num_rounds_input.set_readonly(false);
            self.num_rounds_input.set_enabled(true);
            self.num_growing_objects_input.set_readonly(false);
            self.num_growing_objects_input.set_enabled(true);
            self.extra_delay_sec_input.set_readonly(false);
            self.extra_delay_sec_input.set_enabled(true);
            self.select_area_btn.set_enabled(true);
        }

        self.select_update_timer.stop();
        self.select_update_timer.start();
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
            "sRGB(149,127,86)-sRGB(209,187,146)",
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
    // TODO: is this needed?
    unsafe {
        winapi::um::shellscalingapi::SetProcessDpiAwareness(
            winapi::um::shellscalingapi::PROCESS_DPI_UNAWARE,
        );
    }

    // print messages in the parent console, if any
    unsafe { winapi::um::wincon::AttachConsole(u32::MAX) };

    nwg::init().expect("Failed to init Native Windows GUI");

    let mut font = nwg::Font::default();
    nwg::Font::builder()
        .family("Segoe UI")
        .size(16)
        .build(&mut font)
        .expect("Failed to build font");

    nwg::Font::set_global_default(Some(font));

    let app = BasicApp::build_ui(Default::default()).expect("Failed to build UI");
    nwg::dispatch_thread_events();
    app.state.lock().unwrap().grower.join().unwrap();
}
