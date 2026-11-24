use super::*;
use gpui_kit::{
    TestAppContext,
    component::{Root, Theme, ThemeMode},
    point, size,
    test::TestWindowExt as _,
};
use sovra_core::{NavTab, models::InterfaceScale};
use sovra_runtime::Client;
use std::sync::Arc;

fn example() -> MapPage {
    let mut page = MapPage::default();
    page.hubs();
    let root = page.node(
        Layer::Files,
        "root:one".into(),
        "Observatory",
        "Granted folder",
    );
    page.member(Layer::Files, &root);
    page.branch(
        &root,
        Some(Branch::Files {
            root: "one".into(),
            after: String::new(),
        }),
    );
    let id = page.node(
        Layer::Artifacts,
        "artifact:one".into(),
        "Starling readings",
        "Saved table",
    );
    page.member(Layer::Artifacts, &id);
    page.subject(&id, "Starling", Layer::Artifacts);
    page
}
#[test]
fn linked_layout_preserves_neighbours_in_a_large_scene() {
    let mut page = MapPage::default();
    page.hubs();
    for i in 0..10_000 {
        let layer = Layer::ALL[i % Layer::ALL.len()];
        let id = page.node(layer, format!("record:{i}"), format!("Record {i}"), "");
        page.member(layer, &id);
        if i > 0 {
            page.link(&id, &format!("record:{}", i - 1), "cites", false);
        }
    }
    let scene = Scene::build(&page, true);
    assert_eq!(scene.nodes.len(), 10_007);
    assert_eq!(scene.edges.len(), 19_999);
    assert!(
        scene
            .nodes
            .iter()
            .all(|n| n.x.is_finite() && n.y.is_finite())
    );
    let index = scene.by_id["record:5000"];
    let shown = scene.visible(&BTreeSet::new(), Some(index), false, true);
    assert_eq!(shown.iter().filter(|v| **v).count(), 3);
    for key in ["record:4999", "record:5000", "record:5001"] {
        assert!(shown[scene.by_id[key]]);
    }
}

#[gpui_kit::test]
fn knowledge_map_fits_the_pane_and_expands_through_the_native_controls(cx: &mut TestAppContext) {
    for (width, height, scale, mode) in [
        (640., 420., 200, ThemeMode::Light),
        (2560., 1440., 100, ThemeMode::Dark),
    ] {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| {
            crate::init(client, cx);
            design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            Theme::change(mode, None, cx);
        });
        let retained = Rc::new(std::cell::OnceCell::new());
        let capture = retained.clone();
        let window = cx.open_window(
            size(Pixels::from(width), Pixels::from(height)),
            move |window, cx| {
                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(0, window, cx);
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell
                });
                capture.set(shell.clone()).ok().unwrap();
                Root::new(shell, window, cx)
            },
        );
        let shell = retained.get().unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("open-knowledge-map", cx);
            window.render_frame(cx);
            let request = std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|c| {
                    if let Command::KnowledgeMap(r) = c {
                        Some(r)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(request.branch, Branch::Overview);
            shell.update(cx, |s, cx| {
                s.observe_knowledge_map(
                    &Event::KnowledgeMapped {
                        request_id: request.id,
                        result: Ok(Arc::new(example())),
                    },
                    cx,
                );
            });
            window.render_frame(cx);
            window.render_frame(cx);
            let canvas = window.find("knowledge-map-canvas").bounds();
            let pane = window.find("all-knowledge-map").bounds();
            assert!(canvas.size.width >= Pixels::from(width) * 0.95);
            assert!(canvas.top() >= pane.top() && canvas.bottom() <= pane.bottom());
            let minimum_share = if scale == 200 { 0.45 } else { 0.85 };
            assert!(
                canvas.size.height >= pane.size.height * minimum_share,
                "{canvas:?} inside {pane:?}"
            );
            assert!(window.find("atlas-fit").visible());
            assert!(window.try_find("sovra-sidebar").is_none());

            let map = &shell.read(cx).knowledge_map;
            let label = (0..map.scene.nodes.len())
                .find(|&i| {
                    window
                        .try_find(("atlas-label", i))
                        .is_some_and(|label| label.visible())
                })
                .expect("one drawn label");
            let anchor = window.find(("atlas-label", label)).bounds().center();
            let frame = map.frame.unwrap();
            let before = frame.to_screen(map.camera, 500., 500.);
            window.scroll(
                ("atlas-label", label),
                gpui_kit::ScrollDelta::Lines(point(0., 3.)),
                cx,
            );
            window.render_frame(cx);
            let zoomed = shell.read(cx).knowledge_map.camera;
            assert!((zoomed.zoom - design::tokens(cx).graph.zoom_step).abs() < f32::EPSILON);
            let after = frame.to_screen(zoomed, 500., 500.);
            let expected = anchor + (before - anchor) * zoomed.zoom;
            assert!(
                (after.x - expected.x).abs() < Pixels::from(1.),
                "{width}: {after:?}, expected {expected:?} at {anchor:?}"
            );
            assert!(
                (after.y - expected.y).abs() < Pixels::from(1.),
                "{width}: {after:?}, expected {expected:?} at {anchor:?}"
            );
            window.scroll(
                "knowledge-map-canvas",
                gpui_kit::ScrollDelta::Pixels(point(Pixels::ZERO, -window.line_height())),
                cx,
            );
            window.render_frame(cx);
            assert!(shell.read(cx).knowledge_map.camera.zoom < zoomed.zoom);
            assert_eq!(window.find("knowledge-map-canvas").bounds(), canvas);
            window.click("atlas-fit", cx);
            window.render_frame(cx);
            assert_eq!(shell.read(cx).knowledge_map.camera, Viewport::default());
            window.click("atlas-filters", cx);
            window.render_frame(cx);
            if scale == 200 {
                assert!(window.find("atlas-filter-scroll").visible());
                window.click("atlas-filters", cx);
                window.render_frame(cx);
                assert_eq!(window.find("knowledge-map-canvas").bounds(), canvas);
                return;
            }
            window.click("atlas-list", cx);
            window.render_frame(cx);
            let index = shell.read(cx).knowledge_map.scene.by_id["root:one"];
            window.click(("atlas-record", index), cx);
            window.render_frame(cx);
            assert!(window.find("atlas-inspector-scroll").visible());
            window.click("atlas-expand", cx);
            let request = std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|c| {
                    if let Command::KnowledgeMap(r) = c {
                        Some(r)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!(matches!(request.branch,Branch::Files{root,..} if root=="one"));
            let mut next = MapPage::default();
            next.file("one", "starling", "Starling.txt");
            shell.update(cx, |s, cx| {
                s.observe_knowledge_map(
                    &Event::KnowledgeMapped {
                        request_id: request.id,
                        result: Ok(Arc::new(next)),
                    },
                    cx,
                );
            });
            window.render_frame(cx);
            assert!(
                shell
                    .read(cx)
                    .knowledge_map
                    .data
                    .nodes
                    .contains_key("artifact:one")
            );
            assert!(
                shell
                    .read(cx)
                    .knowledge_map
                    .data
                    .nodes
                    .contains_key("file:starling")
            );
            window.click(("atlas-layer", Layer::Files as usize), cx);
            window.render_frame(cx);
            assert!(shell.read(cx).knowledge_map.selected.is_none());
            window.click("atlas-close", cx);
            window.render_frame(cx);
            assert!(window.try_find("open-knowledge-map").is_some());
        })
        .unwrap();
    }
}
