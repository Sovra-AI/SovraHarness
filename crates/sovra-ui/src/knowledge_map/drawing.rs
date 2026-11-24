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

fn node_at(
    state: &MapView,
    at: Point<Pixels>,
    window: &Window,
    cx: &Context<Shell>,
) -> Option<usize> {
    let frame = state.frame?;
    let ink = design::tokens(cx).graph;
    let rem = window.rem_size();
    let base = ink.node.to_pixels(rem);
    let hub = ink.node_max.to_pixels(rem);
    let tap = ink.hit.to_pixels(rem) * 0.45;
    let reach = tap.max(hub * 0.5 * state.camera.zoom.sqrt().clamp(0.85, 1.7));
    let scene = &state.scene;
    let screen = |i: usize| {
        let dot = &scene.nodes[i];
        frame.to_screen(state.camera, dot.x, dot.y)
    };
    let first = scene.by_x.partition_point(|&i| screen(i).x < at.x - reach);
    let last = scene.by_x.partition_point(|&i| screen(i).x <= at.x + reach);
    scene.by_x[first..last]
        .iter()
        .copied()
        .filter(|&i| state.shown[i])
        .filter_map(|i| {
            let p = screen(i);
            let distance = (at.x - p.x).abs().max((at.y - p.y).abs());
            let radius = node_diameter(scene, i, base, hub, state.camera.zoom) * 0.5;
            (frame.bounds.contains(&p) && distance <= tap.max(radius)).then_some((i, distance))
        })
        .min_by_key(|&(_, distance)| distance)
        .map(|(i, _)| i)
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
    let hover = state.hover.clone();
    let focus = hover.as_ref().map(|h| h.node).or(selected);
    let quiet = tokens.text_secondary.grayscale();
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
                    shell.knowledge_map.hover = None;
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
                    let lit = Some(e.from) == focus || Some(e.to) == focus;
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
                        let stroke = if active {
                            colour.alpha(0.95)
                        } else if hover.is_some() {
                            quiet.alpha(0.20)
                        } else if selected.is_some() {
                            colour.alpha(0.14)
                        } else {
                            colour.alpha(0.52)
                        };
                        window.paint_path(p, stroke);
                    }
                }
            }
            // Paint the hovered neighbourhood last so it stays visible in dense clusters.
            let background =
                (0..scene.nodes.len()).filter(|&i| !hover.as_ref().is_some_and(|h| h.includes(i)));
            let neighbours = hover.iter().flat_map(|h| h.neighbours.iter().copied());
            for i in background
                .chain(neighbours)
                .chain(hover.iter().map(|h| h.node))
            {
                let dot = &scene.nodes[i];
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
                let muted = hover.as_ref().is_some_and(|h| !h.includes(i));
                let colour = if muted {
                    quiet.alpha(0.48)
                } else {
                    palette[dot.layer as usize]
                };
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
                if !muted && (dot.hub || Some(i) == focus) {
                    let alpha = if hover.as_ref().is_some_and(|h| h.node == i) {
                        0.32
                    } else {
                        0.15
                    };
                    window.paint_quad(disc(diameter * 1.65, colour.alpha(alpha)));
                }
                window.paint_quad(
                    disc(diameter, colour)
                        .border_color(if Some(i) == focus {
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
        .aria_label(
            "Knowledge graph. Hover to highlight connections or select a point to inspect them.",
        )
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
            cx.listener(|shell, e: &gpui_kit::MouseDownEvent, _, cx| {
                shell.knowledge_map.drag = Some((e.position, e.position));
                if shell.knowledge_map.set_hover(None) {
                    cx.notify();
                }
            }),
        )
        .on_hover(cx.listener(|shell, inside: &bool, _, cx| {
            if !inside && shell.knowledge_map.set_hover(None) {
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
                if let Some(i) = node_at(s, e.position, window, cx) {
                    s.select_index(i);
                    cx.notify();
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
    let mut labels: Vec<(usize, Bounds<Pixels>)> = Vec::new();
    if let Some(frame) = state.frame.filter(|_| state.on(OptionKey::Labels)) {
        let scene = &state.scene;
        let mut order = Vec::new();
        if let Some(h) = &state.hover {
            order.push(h.node);
            order.extend(h.neighbours.iter().copied());
        }
        order.extend(selected);
        order.extend(
            scene
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, d)| d.hub)
                .map(|(i, _)| i),
        );
        order.extend(0..scene.nodes.len());
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
                || labels.iter().any(|(_, r)| r.intersects(&bounds))
            {
                continue;
            }
            labels.push((i, bounds));
            area = area.child(
                div()
                    .id(("atlas-label", i))
                    .test_support()
                    .aria_label(dot.label.clone())
                    .absolute()
                    .left(at.x - frame.bounds.left())
                    .top(at.y - frame.bounds.top())
                    .w(width)
                    .h(bounds.size.height)
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
                    .text_color(if state.hover.as_ref().is_some_and(|h| !h.includes(i)) {
                        quiet.alpha(0.70)
                    } else {
                        cx.theme().foreground
                    })
                    .truncate()
                    .child(dot.label.clone())
                    .cursor_pointer()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.knowledge_map.select_index(i);
                        cx.notify();
                    })),
            );
            if labels.len() >= 36 {
                break;
            }
        }
    }
    area.on_mouse_move(
        cx.listener(move |shell, e: &gpui_kit::MouseMoveEvent, window, cx| {
            let state = &mut shell.knowledge_map;
            if e.pressed_button != Some(MouseButton::Left) {
                state.drag = None;
                let next = labels
                    .iter()
                    .find(|(_, bounds)| bounds.contains(&e.position))
                    .map(|&(i, _)| i)
                    .or_else(|| node_at(state, e.position, window, cx));
                if state.set_hover(next) {
                    cx.notify();
                }
                return;
            }
            if let Some((start, last)) = state.drag {
                state.camera.pan += e.position - last;
                state.drag = Some((start, e.position));
                cx.notify();
            }
        }),
    )
}
