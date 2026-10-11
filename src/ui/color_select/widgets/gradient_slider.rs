//! A slider whose track is a colour gradient, with a white bar handle.
//!
//! Used for the HSB and RGB channels of the Select Color panel and, upright,
//! for the hue strip beside the saturation/brightness field. The value is
//! normalised to 0–1; callers map it to their own range.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, tree, Widget};
use iced::advanced::{mouse, renderer, Shell};
use iced::gradient::Linear;
use iced::{
    Background, Border, Color, Element, Event, Gradient, Length, Point, Radians, Rectangle,
    Renderer, Shadow, Size, Theme, Vector,
};

const TRACK_THICKNESS: f32 = 18.0;
const TRACK_RADIUS: f32 = 5.0;
const HANDLE_THICKNESS: f32 = 8.0;
const HANDLE_RADIUS: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

pub struct GradientSlider<'a, Message> {
    value: f32,
    stops: Vec<(f32, Color)>,
    on_change: Box<dyn Fn(f32) -> Message + 'a>,
    orientation: Orientation,
    /// How far one mouse-wheel notch moves the value (0–1 units).
    wheel_step: f32,
    width: Length,
    height: Length,
}

#[derive(Default)]
struct State {
    dragging: bool,
}

impl<'a, Message> GradientSlider<'a, Message> {
    /// `value` is 0–1. `stops` are (offset 0–1, colour) pairs, at most eight.
    pub fn new(
        value: f32,
        stops: Vec<(f32, Color)>,
        on_change: impl Fn(f32) -> Message + 'a,
    ) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            stops,
            on_change: Box::new(on_change),
            orientation: Orientation::Horizontal,
            wheel_step: 0.01,
            width: Length::Fill,
            height: Length::Fixed(26.0),
        }
    }

    pub fn vertical(mut self) -> Self {
        self.orientation = Orientation::Vertical;
        self.width = Length::Fixed(22.0);
        self.height = Length::Fill;
        self
    }

    pub fn wheel_step(mut self, step: f32) -> Self {
        self.wheel_step = step;
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// The span the handle centre travels along, in the slider's main axis.
    fn travel(&self, bounds: Rectangle) -> (f32, f32) {
        let inset = HANDLE_THICKNESS / 2.0;
        match self.orientation {
            Orientation::Horizontal => (bounds.x + inset, bounds.x + bounds.width - inset),
            Orientation::Vertical => (bounds.y + inset, bounds.y + bounds.height - inset),
        }
    }

    fn value_at(&self, bounds: Rectangle, cursor: Point) -> f32 {
        let (start, end) = self.travel(bounds);
        let along = match self.orientation {
            Orientation::Horizontal => cursor.x,
            Orientation::Vertical => cursor.y,
        };
        if end <= start {
            return 0.0;
        }
        ((along - start) / (end - start)).clamp(0.0, 1.0)
    }

    fn track_bounds(&self, bounds: Rectangle) -> Rectangle {
        match self.orientation {
            Orientation::Horizontal => {
                let thickness = TRACK_THICKNESS.min(bounds.height);
                Rectangle {
                    y: bounds.y + (bounds.height - thickness) / 2.0,
                    height: thickness,
                    ..bounds
                }
            }
            Orientation::Vertical => bounds,
        }
    }

    fn gradient(&self) -> Background {
        // Gradient angles run clockwise from "bottom to top": a quarter turn
        // is left to right, a half turn top to bottom.
        let angle = match self.orientation {
            Orientation::Horizontal => std::f32::consts::FRAC_PI_2,
            Orientation::Vertical => std::f32::consts::PI,
        };
        let linear = self
            .stops
            .iter()
            .fold(Linear::new(Radians(angle)), |linear, &(offset, color)| {
                linear.add_stop(offset, color)
            });
        Background::Gradient(Gradient::Linear(linear))
    }
}

impl<Message> Widget<Message, Theme, Renderer> for GradientSlider<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_over(bounds) {
                    state.dragging = true;
                    shell.publish((self.on_change)(self.value_at(bounds, position)));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                if let Some(position) = cursor.position() {
                    shell.publish((self.on_change)(self.value_at(bounds, position)));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => y.signum(),
                };
                if lines != 0.0 {
                    // Scrolling up raises the value on a horizontal slider and
                    // moves the handle up on an upright one.
                    let direction = match self.orientation {
                        Orientation::Horizontal => lines.signum(),
                        Orientation::Vertical => -lines.signum(),
                    };
                    let next = (self.value + direction * self.wheel_step).clamp(0.0, 1.0);
                    shell.publish((self.on_change)(next));
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;

        let bounds = layout.bounds();
        renderer.fill_quad(
            renderer::Quad {
                bounds: self.track_bounds(bounds),
                border: Border {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                    width: 1.0,
                    radius: TRACK_RADIUS.into(),
                },
                ..renderer::Quad::default()
            },
            self.gradient(),
        );

        let (start, end) = self.travel(bounds);
        let centre = start + (end - start) * self.value;
        let handle = match self.orientation {
            Orientation::Horizontal => Rectangle {
                x: centre - HANDLE_THICKNESS / 2.0,
                y: bounds.y,
                width: HANDLE_THICKNESS,
                height: bounds.height,
            },
            Orientation::Vertical => Rectangle {
                x: bounds.x - 3.0,
                y: centre - HANDLE_THICKNESS / 2.0,
                width: bounds.width + 6.0,
                height: HANDLE_THICKNESS,
            },
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: handle,
                border: Border {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                    width: 1.0,
                    radius: HANDLE_RADIUS.into(),
                },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.55),
                    offset: Vector::new(0.0, 1.0),
                    blur_radius: 4.0,
                },
                ..renderer::Quad::default()
            },
            Color::WHITE,
        );
    }
}

impl<'a, Message: 'a> From<GradientSlider<'a, Message>> for Element<'a, Message> {
    fn from(slider: GradientSlider<'a, Message>) -> Self {
        Element::new(slider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slider() -> GradientSlider<'static, f32> {
        GradientSlider::new(0.5, vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], |v| v)
    }

    #[test]
    fn cursor_maps_to_value_inside_handle_travel() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(108.0, 26.0));
        let s = slider();
        assert_eq!(s.value_at(bounds, Point::new(4.0, 10.0)), 0.0);
        assert_eq!(s.value_at(bounds, Point::new(104.0, 10.0)), 1.0);
        assert!((s.value_at(bounds, Point::new(54.0, 10.0)) - 0.5).abs() < 1e-6);
        assert_eq!(s.value_at(bounds, Point::new(-50.0, 10.0)), 0.0);
        assert_eq!(s.value_at(bounds, Point::new(500.0, 10.0)), 1.0);
    }

    #[test]
    fn vertical_slider_reads_the_y_axis() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(22.0, 108.0));
        let s = slider().vertical();
        assert_eq!(s.value_at(bounds, Point::new(11.0, 4.0)), 0.0);
        assert_eq!(s.value_at(bounds, Point::new(11.0, 104.0)), 1.0);
    }
}
