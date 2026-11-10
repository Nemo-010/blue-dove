//! A CPU fallback for hosts with no usable OpenGL.
//!
//! eframe only draws through `glow` or `wgpu`, so a host with no GL driver
//! gets no window at all. This runs the same [`App`] through winit and
//! softbuffer, rasterizing egui's meshes on the CPU. It is slower than the
//! GPU path, and it is only used when that path fails or when `--smoke` asks
//! for it.
//!
//! The window keeps the app's translucency where the X server gives it an
//! ARGB visual and a compositor blends it: softbuffer uses the window's
//! depth, so the alpha byte the renderer writes survives. Without an ARGB
//! visual or a compositor it draws opaque, the same as the GPU path.
//! Everything else the app draws, including the waveform and the album
//! grid, goes through the same egui meshes and comes out the same.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;

use eframe::egui;
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::window::{Window, WindowId};

use crate::app::App;

/// A repaint egui asked for, carried through the event loop.
enum UserEvent {
    /// Draw, or run the logic alone, at `when`.
    Repaint {
        /// When the frame is due.
        when: Instant,
    },
}

/// Runs `app` on the CPU until its window closes.
///
/// `build` is called once the event loop exists, because the repaint callback
/// has to be installed on the context before the app starts asking for
/// frames.
pub fn run<F>(viewport: egui::ViewportBuilder, build: F) -> Result<(), String>
where
    F: FnOnce(&egui::Context) -> App,
{
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|error| format!("cannot open the event loop: {error}"))?;
    let softbuffer_context = softbuffer::Context::new(event_loop.owned_display_handle())
        .map_err(|error| format!("cannot open the software surface: {error}"))?;

    let ctx = egui::Context::default();
    let proxy = event_loop.create_proxy();
    ctx.set_request_repaint_callback(move |info| {
        let when = Instant::now() + info.delay;
        proxy.send_event(UserEvent::Repaint { when }).ok();
    });

    let app = build(&ctx);
    let mut runner = Runner {
        app,
        ctx,
        viewport,
        softbuffer_context,
        state: None,
        deadline: None,
        error: None,
    };
    event_loop
        .run_app(&mut runner)
        .map_err(|error| format!("the event loop failed: {error}"))?;
    runner.app.save_playlists();
    match runner.error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The window and everything that draws into it.
struct State {
    /// The window the app draws into.
    window: Rc<Window>,
    /// The CPU framebuffer the window presents.
    surface: softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>,
    /// Translates winit events into egui input.
    egui_winit: egui_winit::State,
    /// Rasterizes egui's meshes into the framebuffer.
    renderer: EguiSoftwareRender,
    /// The window's geometry, as egui sees it.
    viewport_info: egui::ViewportInfo,
    /// Whether the app asked to close and did not cancel it.
    close_requested: bool,
}

/// The event loop's state, including the app it drives.
struct Runner {
    /// The player being driven.
    app: App,
    /// The context the app draws through.
    ctx: egui::Context,
    /// The window the app asked for.
    viewport: egui::ViewportBuilder,
    /// The display the software surface belongs to.
    softbuffer_context: softbuffer::Context<OwnedDisplayHandle>,
    /// The window and its surface, once the loop has resumed.
    state: Option<State>,
    /// When the next frame is due, if egui asked for one later.
    deadline: Option<Instant>,
    /// The first error that stopped the loop, reported after it returns.
    error: Option<String>,
}

impl Runner {
    /// Creates the window and the surface the first time the loop resumes.
    fn create_state(&self, event_loop: &ActiveEventLoop) -> Result<State, String> {
        let window = Rc::new(
            egui_winit::create_window(&self.ctx, event_loop, &self.viewport)
                .map_err(|error| format!("cannot create the window: {error}"))?,
        );
        let surface = softbuffer::Surface::new(&self.softbuffer_context, window.clone())
            .map_err(|error| format!("cannot create the software surface: {error}"))?;
        let egui_winit = egui_winit::State::new(
            self.ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let mut viewport_info = egui::ViewportInfo::default();
        egui_winit::update_viewport_info(&mut viewport_info, &self.ctx, &window, true);
        Ok(State {
            window,
            surface,
            egui_winit,
            renderer: EguiSoftwareRender::new(ColorFieldOrder::Bgra),
            viewport_info,
            close_requested: false,
        })
    }

    /// Draws a frame, or runs the logic alone when the window is hidden.
    fn repaint(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let hidden =
            state.window.is_visible() == Some(false) || state.window.is_minimized() == Some(true);
        if hidden {
            self.logic_only(event_loop);
        } else {
            state.window.request_redraw();
        }
    }

    /// Runs the player without drawing, for a window the compositor hid.
    ///
    /// The audio thread keeps playing while the window is hidden, so the
    /// track that runs out still has to hand over to the next one.
    fn logic_only(&mut self, event_loop: &ActiveEventLoop) {
        let Some(mut state) = self.state.take() else {
            return;
        };
        egui_winit::update_viewport_info(&mut state.viewport_info, &self.ctx, &state.window, false);
        let raw_input = state.egui_winit.take_egui_input(&state.window);
        let close_requested = raw_input.viewport().close_requested();

        let app = &mut self.app;
        let ctx = &self.ctx;
        let output = ctx.run_logic(&raw_input, |ctx| {
            app.run_logic(ctx, Some(&state.window));
        });

        let commands = output
            .viewport_commands
            .get(&egui::ViewportId::ROOT)
            .cloned()
            .unwrap_or_default();
        if close_requested && !commands.contains(&egui::ViewportCommand::CancelClose) {
            state.close_requested = true;
        }
        state
            .egui_winit
            .handle_platform_output(&state.window, output.platform_output);
        let mut actions = Vec::new();
        egui_winit::process_viewport_commands(
            ctx,
            &mut state.viewport_info,
            commands,
            &state.window,
            &mut actions,
        );
        if state
            .viewport_info
            .events
            .contains(&egui::ViewportEvent::Close)
        {
            state.close_requested = true;
        }
        state.viewport_info.events.clear();

        let close = state.close_requested;
        self.state = Some(state);
        if close {
            event_loop.exit();
        }
    }

    /// Draws one frame and presents it.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(mut state) = self.state.take() else {
            return;
        };
        let result = self.draw_frame(&mut state);
        let close = state.close_requested;
        self.state = Some(state);
        if let Err(error) = result {
            self.error = Some(error);
            event_loop.exit();
        } else if close {
            event_loop.exit();
        }
    }

    /// Runs the app and rasterizes its meshes into the window's buffer.
    fn draw_frame(&mut self, state: &mut State) -> Result<(), String> {
        let size = state.window.inner_size();
        let width = NonZeroU32::new(size.width).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(size.height).unwrap_or(NonZeroU32::MIN);
        state
            .surface
            .resize(width, height)
            .map_err(|error| format!("cannot resize the software surface: {error}"))?;

        egui_winit::update_viewport_info(&mut state.viewport_info, &self.ctx, &state.window, false);
        let raw_input = state.egui_winit.take_egui_input(&state.window);
        let close_requested = raw_input.viewport().close_requested();

        let app = &mut self.app;
        let ctx = &self.ctx;
        let full_output = ctx.run_ui(raw_input, |ui| {
            app.run_logic(ui.ctx(), Some(&state.window));
            app.draw(ui);
        });
        let egui::FullOutput {
            platform_output,
            mut textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
        } = full_output;

        let commands = viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.commands.clone())
            .unwrap_or_default();
        if close_requested && !commands.contains(&egui::ViewportCommand::CancelClose) {
            state.close_requested = true;
        }

        state
            .egui_winit
            .handle_platform_output(&state.window, platform_output);
        let mut actions = Vec::new();
        egui_winit::process_viewport_commands(
            ctx,
            &mut state.viewport_info,
            commands,
            &state.window,
            &mut actions,
        );
        if state
            .viewport_info
            .events
            .contains(&egui::ViewportEvent::Close)
        {
            state.close_requested = true;
        }
        state.viewport_info.events.clear();

        let paint_jobs = ctx.tessellate(shapes, pixels_per_point);
        let State {
            surface, renderer, ..
        } = state;
        let mut buffer = surface
            .buffer_mut()
            .map_err(|error| format!("cannot get the software buffer: {error}"))?;
        let pixels: &mut [u32] = &mut buffer;
        let bytes: &mut [[u8; 4]] = bytemuck::cast_slice_mut(pixels);
        let mut buffer_ref = BufferMutRef::new(bytes, width.get() as usize, height.get() as usize);
        renderer.render(
            &mut buffer_ref,
            &paint_jobs,
            &textures_delta,
            pixels_per_point,
        );
        // The renderer has taken the textures; epaint panics if a delta is
        // dropped unapplied, so mark it handled before it goes out of scope.
        textures_delta.clear();
        buffer
            .present()
            .map_err(|error| format!("cannot present the software buffer: {error}"))?;
        Ok(())
    }
}

impl ApplicationHandler<UserEvent> for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.create_state(event_loop) {
            Ok(state) => {
                // The first frame has to be asked for: nothing has happened
                // in the window yet, so winit will not send one on its own.
                state.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Repaint { when } => {
                if when <= Instant::now() {
                    self.repaint(event_loop);
                } else {
                    // Keep the earliest deadline: a later request must not
                    // push back a frame an earlier one already asked for.
                    let deadline = self.deadline.map_or(when, |pending| pending.min(when));
                    self.deadline = Some(deadline);
                    event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if window_id != state.window.id() {
            return;
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            self.redraw(event_loop);
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            // egui-winit only reports the event; the app reads it from the
            // viewport input, so put it there and let the app decide whether
            // to cancel the close.
            state
                .egui_winit
                .egui_input_mut()
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
            state.window.request_redraw();
            return;
        }
        let response = state.egui_winit.on_window_event(&state.window, &event);
        if response.repaint {
            state.window.request_redraw();
        }
        if matches!(event, WindowEvent::Destroyed) {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.deadline {
            Some(deadline) if Instant::now() >= deadline => {
                self.deadline = None;
                self.repaint(event_loop);
            }
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}
