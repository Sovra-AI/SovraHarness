//! Sleek Sovra Login view (adapted from `SovraSRXProduction/frontend/src/components/auth/login-form.tsx`).

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{
    Icon, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
};

use crate::design::{self, SovraStyled as _, Surface};
use crate::shell::{Shell, logo};

pub(crate) fn render_login_screen(
    email_input: &Entity<InputState>,
    password_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    v_flex()
        .id("sovra-login-screen")
        .test_support()
        .flex_1()
        .min_h_0()
        .w_full()
        .items_center()
        .overflow_y_scroll()
        .p(t.space.large)
        .child(
            v_flex()
                .id("login-glass-card")
                .test_support()
                .w_full()
                .flex_shrink_0()
                .my_auto()
                .max_w(t.layout.dialog_width)
                .sovra_surface(Surface::Glass, cx)
                .items_center()
                .gap(t.space.small)
                .p(t.space.large)
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap(t.space.small)
                        .pb(t.space.small)
                        .child(logo("login-logo", t.layout.logo_hero))
                        .child(design::page_header(
                            "Welcome back",
                            "Sign in to continue to Sovra.",
                            cx,
                        )),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap(t.space.small)
                        .child(
                            Input::new(email_input)
                                .id("login-email-input")
                                .aria_label("Email address")
                                .prefix(
                                    Icon::new(IconName::Mail)
                                        .small()
                                        .text_color(t.text_secondary),
                                ),
                        )
                        .child(
                            Input::new(password_input)
                                .id("login-password-input")
                                .aria_label("Password")
                                .mask_toggle()
                                .prefix(
                                    Icon::new(IconName::Lock)
                                        .small()
                                        .text_color(t.text_secondary),
                                ),
                        ),
                )
                .child(
                    design::primary_button("login-submit-btn", "Sign in to Sovra", cx)
                        .w_full()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.submit_login(cx);
                        })),
                )
                .child(
                    design::button("login-google-btn", "Continue with Google", cx)
                        .w_full()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.submit_login(cx);
                        })),
                ),
        )
}
