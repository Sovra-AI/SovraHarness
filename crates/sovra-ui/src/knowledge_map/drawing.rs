use super::*;
use crate::memory_view::Frame;
use gpui_kit::{
    Bounds, Hsla, MouseButton, PathBuilder, Pixels, Point, ScrollDelta, canvas, fill, point, size,
};

pub(super) fn zoom(
    shell: &mut Shell,
    factor: f32,
    anchor: Option<Point<Pixels>>,
    cx: &mut Context<Shell>,
) {
    let state = &mut shell.knowledge_map;
    if let Some(frame) = state.frame {
        state.camera = frame.zoom_about(
            state.camera,
            anchor.unwrap_or(frame.bounds.center()),
            factor,
            (0.4, 32.),
        );
        cx.notify();
    }
}
fn node_diameter(scene: &Scene, index: usize, base: Pixels, hub: Pixels, zoom: f32) -> Pixels {
    let diameter = if scene.nodes[index].hub {
        hub
    } else {
        let degree = f32::from(
            u8::try_from(scene.adjacent[index].len())
                .unwrap_or(u8::MAX)
                .min(25),
        );
        base * (0.88 + degree.sqrt() * 0.14)
    };
    diameter * zoom.sqrt().clamp(0.85, 1.7)
}

pub(super) fn canvas_view(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let state = &shell.knowledge_map;
    let tokens = design::tokens(cx);
    let ink = tokens.graph;
    let palette = [
        ink.detail,
        ink.version,
        ink.subject,
        ink.chat,
        ink.message,
        ink.saved_from,
        ink.focus,
    ];
    let scene = state.scene.clone();
    let visible = state.shown.clone();
    let selected = state.selected_index();
    let view = state.camera;
    let draw_members = state.on(OptionKey::Membership);
    let draw_links = state.on(OptionKey::Relationships);
    let weak = cx.weak_entity();
    let drawing = canvas(
        move |bounds, window, cx| {
            let frame = Frame {
                bounds,
                pad: ink.hit.to_pixels(window.rem_size()),
            };
            weak.update(cx, |shell, cx| {
                if shell.knowledge_map.frame != Some(frame) {
                    shell.knowledge_map.frame = Some(frame);
                    // Labels are GPUI elements laid out before the canvas is measured.
                    // Rebuild them against the new bounds after this frame is presented.
                    cx.on_next_frame(window, |_, _, cx| cx.notify());
                }
            })
            .ok();
            frame
        },
        move |_, frame, window, _| {
            let rem = window.rem_size();
            let spacing = ink.hit.to_pixels(rem);
            let mut x = frame.bounds.left() + view.pan.x % spacing;
            while x < frame.bounds.right() {
                let mut y = frame.bounds.top() + view.pan.y % spacing;
                while y < frame.bounds.bottom() {
                    window.paint_quad(fill(
                        Bounds::new(point(x, y), size(ink.line, ink.line)),
                        ink.grid,
                    ));
                    y += spacing;
                }
                x += spacing;
            }
            let points: Vec<_> = scene
                .nodes
                .iter()
                .map(|d| frame.to_screen(view, d.x, d.y))
                .collect();
            // Batch straight segments by style. Edges crossing the viewport remain visible.
            for active in [false, true] {
                let mut paths: [Option<PathBuilder>; 7] = Default::default();
                for e in &scene.edges {
                    if !visible[e.from]
                        || !visible[e.to]
                        || (e.membership && !draw_members)
                        || (!e.membership && !draw_links)
                    {
                        continue;
                    }
                    let lit = Some(e.from) == selected || Some(e.to) == selected;
                    if active != lit {
                        continue;
                    }
                    let (a, b) = (points[e.from], points[e.to]);
                    if a.x.max(b.x) < frame.bounds.left()
                        || a.y.max(b.y) < frame.bounds.top()
                        || a.x.min(b.x) > frame.bounds.right()
                        || a.y.min(b.y) > frame.bounds.bottom()
                    {
                        continue;
                    }
                    let p = paths[scene.nodes[e.from].layer as usize].get_or_insert_with(|| {
                        PathBuilder::stroke(if active {
                            ink.line_focus
                        } else {
                            ink.line * 0.65
                        })
                    });
                    p.move_to(a);
                    p.line_to(b);
                }
                for (path, colour) in paths.into_iter().zip(palette) {
                    if let Some(p) = path
                        && let Ok(p) = p.build()
                    {
                        window.paint_path(
                            p,
                            colour.alpha(if active {
                                0.95
                            } else if selected.is_some() {
                                0.14
                            } else {
                                0.52
                            }),
                        );
                    }
                }
            }
            for (i, dot) in scene.nodes.iter().enumerate() {
                if !visible[i] || !frame.bounds.contains(&points[i]) {
                    continue;
                }
                let diameter = node_diameter(
                    &scene,
                    i,
                    ink.node.to_pixels(rem),
                    ink.node_max.to_pixels(rem),
                    view.zoom,
                );
                let colour = palette[dot.layer as usize];
                let disc = |side: Pixels, colour: Hsla| {
                    fill(
                        Bounds::new(
                            point(points[i].x - side * 0.5, points[i].y - side * 0.5),
                            size(side, side),
                        ),
                        colour,
                    )
                    .corner_radii(side * 0.5)
                };
                if dot.hub || Some(i) == selected {
                    window.paint_quad(disc(diameter * 1.65, colour.alpha(0.15)));
                }
                window.paint_quad(
                    disc(diameter, colour)
                        .border_color(if Some(i) == selected {
                            ink.focus
                        } else {
                            ink.backdrop
                        })
                        .border_widths(ink.ring),
                );
            }
        },
    )
    .absolute()
    .size_full();
    let mut area = div()
        .id("knowledge-map-canvas")
        .test_support()
        .aria_label("Knowledge graph. Select a point to inspect its connections.")
        .relative()
        .w_full()
        .min_w_0()
        .flex_1()
        .min_h_0()
        .rounded(tokens.radius.panel)
        .border_1()
        .border_color(cx.theme().border)
        .bg(ink.backdrop)
        .overflow_hidden()
        .cursor_grab()
        .child(drawing)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|shell, e: &gpui_kit::MouseDownEvent, _, _| {
                shell.knowledge_map.drag = Some((e.position, e.position));
            }),
        )
        .on_mouse_move(cx.listener(|shell, e: &gpui_kit::MouseMoveEvent, _, cx| {
            if e.pressed_button != Some(MouseButton::Left) {
                shell.knowledge_map.drag = None;
                return;
            }
            let state = &mut shell.knowledge_map;
            if let Some((start, last)) = state.drag {
                state.camera.pan += e.position - last;
                state.drag = Some((start, e.position));
                cx.notify();
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|shell, e: &gpui_kit::MouseUpEvent, window, cx| {
                let s = &mut shell.knowledge_map;
                let Some((start, _)) = s.drag.take() else {
                    return;
                };
                let ink = design::tokens(cx).graph;
                let rem = window.rem_size();
                let tap = ink.hit.to_pixels(rem) * 0.45;
                if (start.x - e.position.x)
                    .abs()
                    .max((start.y - e.position.y).abs())
                    > tap
                {
                    return;
                }
                if let Some(frame) = s.frame {
                    let nearest = s
                        .scene
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| s.shown[*i])
                        .filter_map(|(i, d)| {
                            let p = frame.to_screen(s.camera, d.x, d.y);
                            let dist = (p.x - e.position.x).abs().max((p.y - e.position.y).abs());
                            let radius = node_diameter(
                                &s.scene,
                                i,
                                ink.node.to_pixels(rem),
                                ink.node_max.to_pixels(rem),
                                s.camera.zoom,
                            ) * 0.5;
                            (dist <= tap.max(radius)).then_some((i, dist))
                        })
                        .min_by(|a, b| a.1.cmp(&b.1));
                    if let Some((i, _)) = nearest {
                        s.select_index(i);
                        cx.notify();
                    }
                }
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|shell, _, _, _| shell.knowledge_map.drag = None),
        )
        .on_scroll_wheel(
            cx.listener(|shell, e: &gpui_kit::ScrollWheelEvent, window, cx| {
                let lines = match e.delta {
                    ScrollDelta::Pixels(d) => d.y / window.line_height(),
                    ScrollDelta::Lines(d) => d.y,
                };
                // A three-line wheel notch matches one zoom-button step. Pixel
                // deltas retain their fraction of that step on high-resolution pads.
                let factor = design::tokens(cx).graph.zoom_step.powf(lines / 3.);
                zoom(shell, factor, Some(e.position), cx);
                cx.stop_propagation();
            }),
        )
        .on_pinch(cx.listener(|shell, e: &gpui_kit::PinchEvent, _, cx| {
            zoom(shell, 1. + e.delta, Some(e.position), cx);
            cx.stop_propagation();
        }));
    if !state.on(OptionKey::Labels) {
        return area;
    }
    if let Some(frame) = state.frame {
        let scene = &state.scene;
        let mut order = Vec::new();
        if let Some(i) = selected {
            order.push(i);
        }
        order.extend(
            scene
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, d)| d.hub)
                .map(|(i, _)| i),
        );
        order.extend(0..scene.nodes.len());
        let mut occupied: Vec<Bounds<Pixels>> = Vec::new();
        // Label occupancy bounds both shaping and element count regardless of graph size.
        let unit = frame.pad;
        for i in order {
            if !state.shown[i] {
                continue;
            }
            let dot = &scene.nodes[i];
            let spot = frame.to_screen(view, dot.x, dot.y);
            let width = unit
                * (f32::from(
                    u8::try_from(dot.label.chars().count())
                        .unwrap_or(u8::MAX)
                        .min(35),
                ) * 0.27
                    + 0.85);
            let at = point(
                (spot.x - width * 0.5).max(frame.bounds.left()),
                spot.y
                    + unit * if dot.hub { 0.65 } else { 0.32 } * view.zoom.sqrt().clamp(0.85, 1.7),
            );
            let bounds = Bounds::new(at, size(width, unit * 0.86));
            if !frame.bounds.contains(&spot)
                || bounds.right() > frame.bounds.right()
                || bounds.bottom() > frame.bounds.bottom()
                || occupied.iter().any(|r| r.intersects(&bounds))
            {
                continue;
            }
            occupied.push(bounds);
            area = area.child(
                div()
                    .id(("atlas-label", i))
                    .test_support()
                    .aria_label(dot.label.clone())
                    .absolute()
                    .left(at.x - frame.bounds.left())
                    .top(at.y - frame.bounds.top())
                    .w(width)
                    .rounded(tokens.radius.small)
                    .px(tokens.space.hair)
                    .bg(if dot.hub {
                        ink.label
                    } else {
                        ink.backdrop.alpha(0.94)
                    })
                    .text_center()
                    .sovra_text(
                        if dot.hub {
                            TextRole::SectionTitle
                        } else {
                            TextRole::Body
                        },
                        cx,
                    )
                    .text_color(cx.theme().foreground)
                    .truncate()
                    .child(dot.label.clone())
                    .cursor_pointer()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.knowledge_map.select_index(i);
                        cx.notify();
                    })),
            );
            if occupied.len() >= 36 {
                break;
            }
        }
    }
    area
}
