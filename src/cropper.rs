use super::screenshot::{Rectangle, Screenshot};
use glium::{
    self,
    glutin::{
        dpi::LogicalPosition, os::windows::WindowExt, ContextBuilder, ElementState, Event,
        EventsLoop, KeyboardInput, ModifiersState, MouseButton, VirtualKeyCode, WindowBuilder,
        WindowEvent,
    },
    implement_vertex,
    index::{IndexBuffer, PrimitiveType},
    program,
    program::Program,
    texture::{MipmapsOption, RawImage2d, SrgbTexture2d},
    uniform,
    vertex::VertexBuffer,
    Blend, Display, DrawParameters, Surface,
};
use winapi::{shared::windef::HWND, um::winuser::SetForegroundWindow};

#[derive(Debug, Copy, Clone)]
struct Vertex {
    pos: [f32; 2],
}

implement_vertex!(Vertex, pos);

struct CropperPrograms {
    full_quad_tex: Program,
    sub_quad_tex: Program,
}

struct CroppingContext<'a> {
    snap: &'a Screenshot,
    snap_tex: SrgbTexture2d,

    region: Option<Rectangle<f64>>,
}

pub struct Cropper {
    events_loop: EventsLoop,
    display: Display,

    vbo: VertexBuffer<Vertex>,
    index_buffer: IndexBuffer<u16>,
    programs: CropperPrograms,
}

impl std::fmt::Debug for Cropper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cropper")?;
        Ok(())
    }
}

impl Cropper {
    pub fn new() -> Cropper {
        let events_loop = EventsLoop::new();

        let display = Display::new(
            WindowBuilder::new()
                .with_title("Screenshot")
                .with_visibility(false)
                .with_always_on_top(true)
                .with_decorations(false)
                .with_resizable(false),
            ContextBuilder::new().with_vsync(true),
            &events_loop,
        )
        .unwrap();

        Cropper {
            // create a fullscreen quad VBO
            vbo: VertexBuffer::new(
                &display,
                &[
                    Vertex { pos: [0.0, 0.0] },
                    Vertex { pos: [1.0, 0.0] },
                    Vertex { pos: [0.0, 1.0] },
                    Vertex { pos: [1.0, 1.0] },
                ],
            )
            .unwrap(),

            // indices for the VBO
            index_buffer: IndexBuffer::new(
                &display,
                PrimitiveType::TriangleStrip,
                &[0_u16, 1, 2, 3],
            )
            .unwrap(),

            // all the programs we need
            programs: CropperPrograms {
                full_quad_tex: program!(&display,
                    140 => {
                        vertex: include_str!("shaders/full_quad_tex/140.vs"),
                        fragment: include_str!("shaders/full_quad_tex/140.fs"),
                    }
                )
                .unwrap(),

                sub_quad_tex: program!(&display,
                    140 => {
                        vertex: include_str!("shaders/sub_quad_tex/140.vs"),
                        fragment: include_str!("shaders/sub_quad_tex/140.fs"),
                    }
                )
                .unwrap(),
            },

            events_loop,
            display,
        }
    }

    pub fn apply(&mut self, snap: &Screenshot) -> Result<Option<Rectangle<f64>>, ()> {
        self.display
            .gl_window()
            .window()
            .set_max_dimensions(Some((snap.bounds.w as u32, snap.bounds.h as u32).into()));
        self.display
            .gl_window()
            .window()
            .set_min_dimensions(Some((snap.bounds.w as u32, snap.bounds.h as u32).into()));
        self.display
            .gl_window()
            .window()
            .set_position((snap.bounds.x, snap.bounds.y).into());
        self.display.gl_window().window().show();

        unsafe {
            SetForegroundWindow(self.display.gl_window().window().get_hwnd() as HWND);
        }

        let mut context = CroppingContext {
            region: None,

            snap_tex: SrgbTexture2d::with_mipmaps(
                &self.display,
                RawImage2d::from_raw_rgb(
                    // BGR => RGB
                    snap.bgra
                        .chunks_exact(3)
                        .flat_map(|p| [p[2], p[1], p[0]])
                        .collect(),
                    (snap.bounds.w as u32, snap.bounds.h as u32),
                ),
                MipmapsOption::NoMipmap,
            )
            .unwrap(),
            snap,
        };

        // becomes true whenever the window should close
        let mut closed = false;

        // where the left mouse button was pressed
        let mut left_press: Option<(f64, f64)> = None;

        // tracks the position of the cursor
        let mut cursor_pos = (0.0, 0.0);

        // empty the event queue
        self.events_loop.poll_events(|_| ());

        // main loop
        while !closed {
            let mut frame = self.display.draw();
            self.render(&mut frame, &mut context);
            frame.finish().unwrap();

            #[allow(clippy::single_match)]
            self.events_loop.poll_events(|e| match e {
                Event::WindowEvent { event, .. } => match event {
                    // kill process
                    WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                virtual_keycode: Some(VirtualKeyCode::Q),
                                state: ElementState::Pressed,
                                modifiers:
                                    ModifiersState {
                                        ctrl: true,
                                        shift: true,
                                        ..
                                    },
                                ..
                            },
                        ..
                    } => closed = true,

                    // cancel screenshot
                    WindowEvent::CloseRequested
                    | WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                virtual_keycode: Some(VirtualKeyCode::Escape),
                                state: ElementState::Pressed,
                                ..
                            },
                        ..
                    } => {
                        context.region = None;
                        closed = true
                    }

                    WindowEvent::CursorMoved {
                        position: LogicalPosition { x, y },
                        ..
                    } => {
                        cursor_pos = (x, y);

                        if let Some((px, py)) = left_press {
                            context.region = Some(Rectangle {
                                x: px.min(x),
                                y: py.min(y),
                                w: (px - x).abs(),
                                h: (py - y).abs(),
                            });
                        }
                    }

                    WindowEvent::MouseInput { button, state, .. } => match (button, state) {
                        (MouseButton::Left, ElementState::Released) => closed = true,
                        (MouseButton::Left, ElementState::Pressed) => left_press = Some(cursor_pos),
                        _ => (),
                    },

                    // other window events
                    _ => (),
                },

                // other events
                _ => (),
            });
        }

        self.display.gl_window().window().hide();
        Ok(context.region)
    }

    fn render(&mut self, frame: &mut glium::Frame, ctx: &mut CroppingContext) {
        let draw_params = DrawParameters {
            blend: Blend::alpha_blending(),
            ..Default::default()
        };

        // clear to black
        frame.clear_color(0.0, 0.0, 0.0, 1.0);

        // base pass
        let uniforms = uniform! {
            tex: &ctx.snap_tex,
            opacity: 0.5f32,
        };

        frame
            .draw(
                &self.vbo,
                &self.index_buffer,
                &self.programs.full_quad_tex,
                &uniforms,
                &draw_params,
            )
            .unwrap();

        // active region pass
        if let Some(areg) = ctx.region {
            let uniforms = uniform! {
                tex: &ctx.snap_tex,
                opacity: 0.8f32,
                bounds: [
                    (areg.x as f32) / (ctx.snap.bounds.w as f32),
                    1.0 - (areg.y as f32) / (ctx.snap.bounds.h as f32),
                    (areg.w as f32) / (ctx.snap.bounds.w as f32),
                    -(areg.h as f32) / (ctx.snap.bounds.h as f32)
                ],
            };

            frame
                .draw(
                    &self.vbo,
                    &self.index_buffer,
                    &self.programs.sub_quad_tex,
                    &uniforms,
                    &draw_params,
                )
                .unwrap();
        }
    }
}
