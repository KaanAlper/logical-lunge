use anyhow::Context;
use std::time;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BRUSH_PROPERTIES, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_ROUNDED_RECT, ID2D1Brush,
    ID2D1Multithread, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::core::Interface;
use windows_numerics::Matrix3x2;

use crate::APP_STATE;
use crate::animations::{AnimType, Animations};
use crate::border_config::BorderConfig;
use crate::colors::ColorBrush;
use crate::effects::Effects;
use crate::render_backend::{RenderBackend, RenderBackendConfig, TARGET_BITMAP_PROPS};
use crate::utils::{
    StandaloneWindowsError, T_E_UNINIT, ToWindowsResult, WindowsCompatibleError,
    WindowsCompatibleResult, WindowsContext, WriteLockable,
};
use crate::window_border::WindowState;

#[derive(Debug, Default)]
pub struct BorderDrawer {
    pub stroke_width: i32,
    // This is WriteLockable so it doesn't accidentally change when the tracking window is in
    // the snapped/arranged state where the borders are supposed to remain square
    pub corner_radius: WriteLockable<f32>,
    pub render_backend: RenderBackend,
    pub active_color: ColorBrush,
    pub inactive_color: ColorBrush,
    pub animations: Animations,
    pub effects: Effects,
    pub last_render_time: Option<time::Instant>,
    pub last_anim_time: Option<time::Instant>,
    /// Logical Lunge: an animation step was computed but not drawn yet (the frame came early)
    pub unrendered: bool,
    /// Logical Lunge: the inactive dim (illogical-impulse's dim_inactive): black over the window's inside at alpha
    /// `dim`, which moves to `dim_target` over [`DIM_FADE_MS`]. The border window already sits right above its
    /// window and lets clicks through, so this costs no extra window.
    pub dim_strength: f32,
    pub dim: f32,
    pub dim_target: f32,
    dim_brush: Option<ID2D1SolidColorBrush>,
    /// Logical Lunge: a floating window's shadow, as illogical-impulse's (Hyprland shadow range 20, render_power 10,
    /// offset 0 2, color 12.5 % black): alpha 0.125 * (1 - d / range)^10 at d px outside the border. 0 = none.
    /// Drawn as 1 px rings, each at its own alpha (exact, no stacking), with the black brush of the dim.
    pub shadow_range: f32,
    pub shadow_offset: f32,
}

/// illogical-impulse's dim fade (Hyprland fadeDim)
const DIM_FADE_MS: f32 = 800.0;

impl BorderDrawer {
    pub fn configure_appearance(&mut self, config: &BorderConfig, dpi: u32, tracking_window: HWND) {
        let stroke_width = config.width_at(dpi);
        let corner_radius = config.radius_at(stroke_width, dpi, tracking_window);

        self.stroke_width = stroke_width;
        self.corner_radius = WriteLockable::new(corner_radius);
        self.active_color = config.active_color.to_color_brush(true);
        self.inactive_color = config.inactive_color.to_color_brush(false);
        self.animations = config.animations.to_animations();
        self.effects = config.effects.to_effects(dpi);
        self.dim_strength = config.inactive_dim.clamp(0.0, 0.5);
        let floats = config.floating_shadow && crate::is_floating(tracking_window);
        self.shadow_range = if floats { (20.0 * dpi as f32 / 96.0).round() } else { 0.0 };
        self.shadow_offset = if floats { (2.0 * dpi as f32 / 96.0).round() } else { 0.0 };
        self.dim_target = self.dim_target.min(self.dim_strength);
        self.dim = self.dim.min(self.dim_strength);
    }

    /// Sets where the dim goes; true when that changed. Without animations it goes there at once.
    pub fn set_dim(&mut self, dimmed: bool, border_window: HWND) -> bool {
        let target = if dimmed { self.dim_strength } else { 0.0 };
        if target == self.dim_target {
            return false;
        }
        self.dim_target = target;
        if self.animations.active.is_empty() && self.animations.inactive.is_empty() {
            self.dim = target;
        } else {
            self.set_anims_timer_if_needed(border_window);
        }
        true
    }

    /// The dim over the inside of the border (`inner`), under the border's own line.
    fn paint_dim(&self, inner: &D2D1_ROUNDED_RECT, renderer: &ID2D1RenderTarget) {
        if self.dim <= 0.0 {
            return;
        }
        if let Some(brush) = self.dim_brush.as_ref() {
            unsafe { brush.SetOpacity(self.dim) };
            self.fill_rectangle(inner, renderer, brush);
        }
    }

    /// The shadow around `outer` (the border's outer edge, its corner radius `radius`), `shadow_offset` lower. The
    /// rings closer than the offset would cross the window's top: those are drawn below its top corners only.
    fn paint_shadow(&self, outer: &D2D_RECT_F, radius: f32, renderer: &ID2D1RenderTarget) {
        let range = self.shadow_range;
        let Some(brush) = self.dim_brush.as_ref().filter(|_| range > 0.0) else { return };
        let off = self.shadow_offset;
        let mut d = 0.0;
        while d < range {
            let alpha = 0.125 * (1.0 - d / range).powi(10);
            if alpha < 0.002 {
                break;
            }
            let g = d + 0.5;
            let ring = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left: outer.left - g, top: outer.top + off - g, right: outer.right + g, bottom: outer.bottom + off + g },
                radiusX: radius + g,
                radiusY: radius + g,
            };
            unsafe {
                brush.SetOpacity(alpha);
                if d < off {
                    renderer.PushAxisAlignedClip(
                        &D2D_RECT_F { left: f32::MIN, top: outer.top + off + radius, right: f32::MAX, bottom: f32::MAX },
                        windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                }
                renderer.DrawRoundedRectangle(&ring, brush, 1.0, None);
                if d < off {
                    renderer.PopAxisAlignedClip();
                }
            }
            d += 1.0;
        }
    }

    fn inner_rect(stroke_rect: &D2D1_ROUNDED_RECT, half_stroke_width: f32) -> D2D1_ROUNDED_RECT {
        D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: stroke_rect.rect.left + half_stroke_width,
                top: stroke_rect.rect.top + half_stroke_width,
                right: stroke_rect.rect.right - half_stroke_width,
                bottom: stroke_rect.rect.bottom - half_stroke_width,
            },
            radiusX: (stroke_rect.radiusX - half_stroke_width).max(0.0),
            radiusY: (stroke_rect.radiusY - half_stroke_width).max(0.0),
        }
    }

    pub fn init(
        &mut self,
        width: u32,
        height: u32,
        border_window: HWND,
        bounds: D2D_RECT_F,
        render_backend_config: RenderBackendConfig,
    ) -> WindowsCompatibleResult<()> {
        // Drop our current render backend to avoid issues with recreating existing resources when
        // calling init() multiple times in a row
        self.render_backend = RenderBackend::None;

        self.render_backend = render_backend_config
            .to_render_backend(width, height, border_window, self.effects.is_enabled())
            .windows_context("could not initialize render backend in init()")?;

        let renderer: &ID2D1RenderTarget = match self.render_backend {
            RenderBackend::V2(ref backend) => &backend.d2d_context,
            RenderBackend::Legacy(ref backend) => &backend.render_target,
            RenderBackend::None => {
                return Err(WindowsCompatibleError::Standalone(
                    StandaloneWindowsError::new(T_E_UNINIT, "render backend is None"),
                ));
            }
        };

        // We will adjust opacity later. For now, we set it to 0.
        let brush_properties = D2D1_BRUSH_PROPERTIES {
            opacity: 0.0,
            transform: Matrix3x2::identity(),
        };
        self.active_color
            .init_brush(renderer, &bounds, &brush_properties)?;
        self.inactive_color
            .init_brush(renderer, &bounds, &brush_properties)?;
        // SAFETY: `renderer` is the live render target just created.
        self.dim_brush = Some(unsafe { renderer.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None)? });

        if self.render_backend.supports_effects() {
            self.effects
                .init_command_lists_if_enabled(&self.render_backend)
                .windows_context("could not initialize command list")?;
        }

        Ok(())
    }

    pub fn uninit(&mut self) {
        self.render_backend = RenderBackend::None;
        let _ = self.active_color.take_brush();
        let _ = self.inactive_color.take_brush();
        self.dim_brush = None;
        let _ = self.effects.take_active_command_list();
        let _ = self.effects.take_inactive_command_list();
    }

    pub fn resize_renderer(&mut self, width: u32, height: u32) -> WindowsCompatibleResult<()> {
        self.render_backend
            .resize(width, height, self.effects.is_enabled())
            .windows_context("could not update render resources")?;

        if self.render_backend.supports_effects() {
            self.effects
                .init_command_lists_if_enabled(&self.render_backend)
                .context("could not initialize command lists")
                .to_windows_result(T_E_UNINIT)?;
        }

        Ok(())
    }

    /// Renders a border onto the internal bitmap along the inside edge of the bounds. Effects
    /// (e.g. glow) are drawn outside, so the caller should pad the bounds if needed to prevent clipping.
    ///
    /// NOTE: bound coordinates should be specified relative to the bitmap, not the screen.
    pub fn render(
        &mut self,
        bounds: D2D_RECT_F,
        window_state: WindowState,
    ) -> WindowsCompatibleResult<()> {
        self.last_render_time = Some(time::Instant::now());

        // Direct2D draws a stroke centered along a given rect, but we want it to be drawn on the
        // inside of 'bounds'. To achieve this, we pad it by half the stroke width.
        let half_stroke_width = self.stroke_width as f32 / 2.0;
        let stroke_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: bounds.left + half_stroke_width,
                top: bounds.top + half_stroke_width,
                right: bounds.right - half_stroke_width,
                bottom: bounds.bottom - half_stroke_width,
            },
            radiusX: *self.corner_radius.get(),
            radiusY: *self.corner_radius.get(),
        };

        // Note that Rust's borrow checker prevents passing the render backend from the match arm,
        // so I'll need to grab it from within the respective functions instead
        match self.render_backend {
            RenderBackend::V2(_) if self.effects.should_apply(window_state) => {
                self.render_v2_with_effects(stroke_rect, bounds, window_state)?
            }
            RenderBackend::V2(_) => self.render_v2(stroke_rect, bounds, window_state)?,
            RenderBackend::Legacy(_) => self.render_legacy(stroke_rect, bounds, window_state)?,
            RenderBackend::None => {
                return Err(WindowsCompatibleError::Standalone(
                    StandaloneWindowsError::new(T_E_UNINIT, "render_backend is None"),
                ));
            }
        }

        Ok(())
    }

    fn render_legacy(
        &mut self,
        stroke_rect: D2D1_ROUNDED_RECT,
        bounds: D2D_RECT_F,
        window_state: WindowState,
    ) -> WindowsCompatibleResult<()> {
        let RenderBackend::Legacy(ref backend) = self.render_backend else {
            return Err(WindowsCompatibleError::Standalone(
                StandaloneWindowsError::new(
                    T_E_UNINIT,
                    "could not get render_backend within render()",
                ),
            ));
        };
        let render_target = &backend.render_target;

        unsafe {
            // Determine which color should be drawn on top (for color fade animation)
            let (bottom_color, top_color) = match window_state {
                WindowState::Active => (&self.inactive_color, &self.active_color),
                WindowState::Inactive => (&self.active_color, &self.inactive_color),
            };

            render_target.BeginDraw();
            render_target.Clear(None);

            self.paint_dim(&Self::inner_rect(&stroke_rect, self.stroke_width as f32 / 2.0), render_target);
            self.paint_colors(bottom_color, top_color, &bounds, render_target, &|brush| {
                self.draw_rectangle(&stroke_rect, render_target, brush)
            })?;

            render_target.EndDraw(None, None)?;
        }

        Ok(())
    }

    fn render_v2(
        &mut self,
        stroke_rect: D2D1_ROUNDED_RECT,
        bounds: D2D_RECT_F,
        window_state: WindowState,
    ) -> WindowsCompatibleResult<()> {
        let RenderBackend::V2(ref backend) = self.render_backend else {
            return Err(WindowsCompatibleError::Standalone(
                StandaloneWindowsError::new(
                    T_E_UNINIT,
                    "could not get render_backend within render()",
                ),
            ));
        };
        let d2d_context = &backend.d2d_context;

        unsafe {
            // Determine which color should be drawn on top (for color fade animation)
            let (bottom_color, top_color) = match window_state {
                WindowState::Active => (&self.inactive_color, &self.active_color),
                WindowState::Inactive => (&self.active_color, &self.inactive_color),
            };

            // We're about to use DirectComposition which means we will be using the underlying
            // Direct3D objects without Direct2D's knowledge. To avoid resource access conflict, we
            // must explicitly acquire a lock. Read the following article for more info:
            // https://learn.microsoft.com/en-us/windows/win32/direct2d/multi-threaded-direct2d-apps
            let d2d_multithread: ID2D1Multithread = APP_STATE
                .render_factory
                .cast()
                .windows_context("d2d_multithread")?;
            let _d2d_lock = crate::utils::D2DLock::enter(&d2d_multithread);

            // Set d2d_context's target back to the target_bitmap so we can draw to the display
            let mut point = POINT::default();
            let dxgi_surface: IDXGISurface = backend
                .d_comp_surface
                .BeginDraw(None, &mut point)
                .windows_context("dxgi_surface")?;
            let target_bitmap = d2d_context
                .CreateBitmapFromDxgiSurface(&dxgi_surface, Some(&TARGET_BITMAP_PROPS))
                .windows_context("target_bitmap")?;
            d2d_context.SetTarget(&target_bitmap);

            // Draw to the target_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            self.paint_shadow(&bounds, stroke_rect.radiusX + self.stroke_width as f32 / 2.0, d2d_context);
            self.paint_dim(&Self::inner_rect(&stroke_rect, self.stroke_width as f32 / 2.0), d2d_context);
            self.paint_colors(bottom_color, top_color, &bounds, d2d_context, &|brush| {
                self.draw_rectangle(&stroke_rect, d2d_context, brush)
            })?;

            d2d_context.EndDraw(None, None)?;

            d2d_context.SetTarget(None);
            backend
                .d_comp_surface
                .EndDraw()
                .windows_context("d_comp_surface.EndDraw()")?;
            backend
                .d_comp_device
                .Commit()
                .windows_context("d_comp_device.Commit()")?;

        }

        Ok(())
    }

    fn render_v2_with_effects(
        &mut self,
        stroke_rect: D2D1_ROUNDED_RECT,
        bounds: D2D_RECT_F,
        window_state: WindowState,
    ) -> WindowsCompatibleResult<()> {
        let RenderBackend::V2(ref backend) = self.render_backend else {
            return Err(WindowsCompatibleError::Standalone(
                StandaloneWindowsError::new(
                    T_E_UNINIT,
                    "could not get render_backend within render()",
                ),
            ));
        };
        let d2d_context = &backend.d2d_context;

        let half_stroke_width = stroke_rect.rect.left - bounds.left;

        unsafe {
            // Determine which color should be drawn on top (for color fade animation)
            let (bottom_color, top_color) = match window_state {
                WindowState::Active => (&self.inactive_color, &self.active_color),
                WindowState::Inactive => (&self.active_color, &self.inactive_color),
            };

            // Create a rect that covers up to the outer edge of the border
            let border_outer_rect = D2D1_ROUNDED_RECT {
                rect: bounds,
                radiusX: stroke_rect.radiusX + half_stroke_width,
                radiusY: stroke_rect.radiusY + half_stroke_width,
            };

            // Set the d2d_context target to the border_bitmap
            let border_bitmap = backend
                .border_bitmap
                .as_ref()
                .context("could not get border_bitmap")
                .to_windows_result(T_E_UNINIT)?;
            d2d_context.SetTarget(border_bitmap);

            // Draw to the border_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            // We use filled rectangles here because it helps make the effects more visible.
            // Additionally, if someone sets the stroke width to 0, the effects will still be
            // visible (whereas they wouldn't be if we used a hollow rectangle).
            self.paint_colors(bottom_color, top_color, &bounds, d2d_context, &|brush| {
                self.fill_rectangle(&border_outer_rect, d2d_context, brush)
            })?;

            d2d_context.EndDraw(None, None)?;
        }

        unsafe {
            // Set the d2d_context target to the mask_bitmap to create an alpha mask
            let mask_bitmap = backend
                .mask_bitmap
                .as_ref()
                .context("could not get mask_bitmap")
                .to_windows_result(T_E_UNINIT)?;
            d2d_context.SetTarget(mask_bitmap);

            // Create a rect that covers up to the inner edge of the border
            // This rect is used to mask out the inner portion of the border
            let border_inner_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: stroke_rect.rect.left + half_stroke_width,
                    top: stroke_rect.rect.top + half_stroke_width,
                    right: stroke_rect.rect.right - half_stroke_width,
                    bottom: stroke_rect.rect.bottom - half_stroke_width,
                },
                radiusX: stroke_rect.radiusX - half_stroke_width,
                radiusY: stroke_rect.radiusY - half_stroke_width,
            };

            // Create a 100% opaque brush because our active/inactive colors' brushes might not be
            let opaque_brush = d2d_context.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )?;

            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            self.fill_rectangle(&border_inner_rect, d2d_context, &opaque_brush);

            d2d_context.EndDraw(None, None)?;
        }

        unsafe {
            // We're about to use DirectComposition which means we will be using the underlying
            // Direct3D objects without Direct2D's knowledge. To avoid resource access conflict, we
            // must explicitly acquire a lock. Read the following article for more info:
            // https://learn.microsoft.com/en-us/windows/win32/direct2d/multi-threaded-direct2d-apps
            let d2d_multithread: ID2D1Multithread = APP_STATE
                .render_factory
                .cast()
                .windows_context("d2d_multithread")?;
            let _d2d_lock = crate::utils::D2DLock::enter(&d2d_multithread);

            // Set d2d_context's target back to the target_bitmap so we can draw to the display
            let mut point = POINT::default();
            let dxgi_surface: IDXGISurface = backend
                .d_comp_surface
                .BeginDraw(None, &mut point)
                .windows_context("dxgi_surface")?;
            let target_bitmap = d2d_context
                .CreateBitmapFromDxgiSurface(&dxgi_surface, Some(&TARGET_BITMAP_PROPS))
                .windows_context("target_bitmap")?;
            d2d_context.SetTarget(&target_bitmap);

            // Retrieve our command list (includes border_bitmap, mask_bitmap, and effects)
            let command_list = self
                .effects
                .get_current_command_list(window_state)
                .to_windows_result(T_E_UNINIT)?;

            // Draw to the target_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            self.paint_shadow(&bounds, stroke_rect.radiusX + half_stroke_width, d2d_context);
            self.paint_dim(&Self::inner_rect(&stroke_rect, half_stroke_width), d2d_context);
            d2d_context.DrawImage(
                command_list,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );

            d2d_context.EndDraw(None, None)?;

            d2d_context.SetTarget(None);
            backend
                .d_comp_surface
                .EndDraw()
                .windows_context("d_comp_surface.EndDraw()")?;
            backend
                .d_comp_device
                .Commit()
                .windows_context("d_comp_device.Commit()")?;

        }

        Ok(())
    }

    // Paints the border's color(s) for the current fade state. Logical Lunge: mid-fade between two
    // solid colors, a single color mixed in OkLab is painted (see `ColorBrush::fade_mix_with`);
    // otherwise the fading-out color is painted under the fading-in one, as before.
    fn paint_colors(
        &self,
        bottom_color: &ColorBrush,
        top_color: &ColorBrush,
        bounds: &D2D_RECT_F,
        renderer: &ID2D1RenderTarget,
        paint: &dyn Fn(&ID2D1Brush),
    ) -> WindowsCompatibleResult<()> {
        if let Some(mixed) = top_color.fade_mix_with(bottom_color) {
            // SAFETY: `renderer` is a live render target inside BeginDraw/EndDraw.
            let brush = unsafe { renderer.CreateSolidColorBrush(&mixed, None)? };
            paint((&brush).into());
            return Ok(());
        }

        for (color, name) in [(bottom_color, "bottom_color"), (top_color, "top_color")] {
            if color.get_opacity().to_windows_result(T_E_UNINIT)? > 0.0 {
                if let ColorBrush::Gradient(gradient) = color {
                    gradient.update_start_end_points(bounds);
                }

                match color.get_brush() {
                    Some(id2d1_brush) => paint(id2d1_brush),
                    None => debug!("ID2D1Brush for {name} has not been created yet"),
                }
            }
        }

        Ok(())
    }

    // NOTE: ID2D1DeviceContext implements From<&ID2D1DeviceContext> for &ID2D1RenderTarget
    fn draw_rectangle(
        &self,
        stroke_rect: &D2D1_ROUNDED_RECT,
        renderer: &ID2D1RenderTarget,
        brush: &ID2D1Brush,
    ) {
        unsafe {
            match stroke_rect.radiusX {
                0.0 => {
                    renderer.DrawRectangle(&stroke_rect.rect, brush, self.stroke_width as f32, None)
                }
                _ => renderer.DrawRoundedRectangle(
                    stroke_rect,
                    brush,
                    self.stroke_width as f32,
                    None,
                ),
            }
        }
    }

    // NOTE: ID2D1DeviceContext implements From<&ID2D1DeviceContext> for &ID2D1RenderTarget
    fn fill_rectangle(
        &self,
        rounded_rect: &D2D1_ROUNDED_RECT,
        renderer: &ID2D1RenderTarget,
        brush: &ID2D1Brush,
    ) {
        unsafe {
            match rounded_rect.radiusX {
                0.0 => renderer.FillRectangle(&rounded_rect.rect, brush),
                _ => renderer.FillRoundedRectangle(rounded_rect, brush),
            }
        }
    }

    pub fn set_anims_timer_if_needed(&mut self, border_window: HWND) {
        self.animations
            .set_timer_if_needed(border_window, &mut self.last_anim_time);
    }

    pub fn destroy_anims_timer(&mut self) {
        self.animations.destroy_timer();
    }

    pub fn animate(&mut self, bounds: D2D_RECT_F, window_state: WindowState) -> anyhow::Result<()> {
        let anim_elapsed = self
            .last_anim_time
            .get_or_insert_with(time::Instant::now)
            .elapsed();
        let render_elapsed = self
            .last_render_time
            .get_or_insert_with(time::Instant::now)
            .elapsed();

        let mut update = false;

        for anim_params in self.animations.get_current(window_state).clone().iter() {
            match anim_params.anim_type {
                AnimType::Spiral | AnimType::ReverseSpiral => {
                    self.animations.animate_spiral(
                        &bounds,
                        &self.active_color,
                        &self.inactive_color,
                        &anim_elapsed,
                        anim_params,
                    );
                    update = true;
                }
                AnimType::Fade => {
                    let correct_active_opacity = if window_state == WindowState::Active {
                        1.0
                    } else {
                        0.0
                    };

                    if self.active_color.get_opacity()? != correct_active_opacity
                        || self.inactive_color.get_opacity()? != 1.0 - correct_active_opacity
                    {
                        self.animations.animate_fade(
                            window_state,
                            &self.active_color,
                            &self.inactive_color,
                            &anim_elapsed,
                            anim_params,
                        )?;
                        update = true;
                    }
                }
            }
        }

        if self.dim != self.dim_target {
            let step = anim_elapsed.as_secs_f32() * 1000.0 / DIM_FADE_MS * self.dim_strength;
            self.dim = if self.dim < self.dim_target {
                (self.dim + step).min(self.dim_target)
            } else {
                (self.dim - step).max(self.dim_target)
            };
            update = true;
        }

        self.last_anim_time = Some(time::Instant::now());

        let render_interval = 1.0 / self.animations.fps as f32;
        let time_diff = render_elapsed.as_secs_f32() - render_interval;
        if update && (time_diff.abs() <= 0.001 || time_diff >= 0.0) {
            self.render(bounds, window_state)?;
            self.unrendered = false;
        } else if update {
            self.unrendered = true;
        }

        // Logical Lunge: nothing left to animate (a fade has reached its colors, and no spiral runs): the timer
        // stops, after drawing the last step if that was skipped. It ran at the display's rate for every shown
        // border all the time: ~1400 wakeups a second with four borders on an idle desktop. A focus change
        // starts it again (update_color), as do a show or a move.
        if !update && !self.animations.continuous() {
            if self.unrendered {
                self.render(bounds, window_state)?;
                self.unrendered = false;
            }
            self.animations.destroy_timer();
        }

        Ok(())
    }
}
