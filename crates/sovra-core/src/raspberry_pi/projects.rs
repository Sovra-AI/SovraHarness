//! Small, bounded hardware lessons. Returned code is never executed by Sovra.

use super::{Board, Family};

impl Board {
    pub fn led_project(&self) -> String {
        if self.family == Family::ComputeModule {
            return self.carrier_project_help("an LED");
        }
        let (wire, code, environment) = if self.family == Family::Pico {
            (
                "Physical pin 20 (GPIO 15) → 330 Ω resistor → LED long leg (+). LED short leg (−) → physical pin 18 (Ground).",
                include_str!("projects/pico_led.py"),
                "Use MicroPython for your exact Pico and select that board in Thonny.",
            )
        } else {
            (
                "Physical pin 11 (GPIO 17) → 330 Ω resistor → LED long leg (+). LED short leg (−) → physical pin 9 (Ground). These positions are shared by the 26-pin and 40-pin Pi headers.",
                include_str!("projects/pi_led.py"),
                "Run Python 3 on the Pi with GPIO Zero from current compatible Raspberry Pi OS. Pi 5 needs a supported GPIO Zero backend such as lgpio; do not copy old RPi.GPIO or fixed gpiochip-number instructions.",
            )
        };
        format!(
            "# {} · Blink an external light\n\nYou need one ordinary low-current LED, a 330 Ω resistor, a breadboard and jumper wires.\n\n1. Shut down and disconnect power before wiring. Disconnect other add-on boards from these pins.\n2. Check pin 1 and your board's map. {wire}\n3. Reconnect power. {environment}\n4. Review and run this code. It flashes ten times and leaves the light off; Stop or Ctrl+C ends it early.\n\n```python\n{code}```\n\nIf it stays dark, stop and unplug before checking LED polarity, the resistor and physical pin numbers. Never substitute a 5 V pin for a GPIO. Opening this project does not drive the hardware.",
            self.name
        )
    }

    pub fn button_project(&self) -> String {
        if self.family == Family::ComputeModule {
            return self.carrier_project_help("a push button");
        }
        let (wire, code, environment) = if self.family == Family::Pico {
            (
                "Connect GPIO 14 (physical pin 19) to one side of a normally-open button, and Ground (physical pin 18) to its other side.",
                include_str!("projects/pico_button.py"),
                "Use MicroPython for your Pico and Thonny.",
            )
        } else {
            (
                "Connect GPIO 17 (physical pin 11) to one side of a normally-open button, and Ground (physical pin 9) to its other side. This works with either P1 revision and the 40-pin header.",
                include_str!("projects/pi_button.py"),
                "Run Python 3 on the Pi with GPIO Zero from compatible Raspberry Pi OS. Use a supported backend such as lgpio on Pi 5.",
            )
        };
        format!(
            "# {} · Read a button\n\nYou need a normally-open push button, a breadboard and jumper wires.\n\n1. Disconnect power and remove the LED project or other hardware from these pins.\n2. {wire} A four-leg button has internally joined pairs: use opposite switching contacts, not two legs already joined together.\n3. No external power wire is needed: the code enables an internal pull-up. Reconnect power. {environment}\n4. Run the code and press/release the button. It reports changes for 30 seconds, then ends. Stop or Ctrl+C ends it early.\n\n```python\n{code}```\n\nAlways pressed usually means the contacts are shorted or the wrong button legs were used. Never pressed usually means an open wire, wrong pin or missing ground. Unplug before changing wires. Sovra only supplies the project; it does not read a pin when you open this guide.",
            self.name
        )
    }

    fn carrier_project_help(&self, part: &str) -> String {
        format!(
            "# {} · Plan a project with {part}\n\n{}\n\n1. Read the carrier's exact name and revision.\n2. Find its GPIO connector diagram, the signal's voltage and a Ground connection. Confirm the signal is available rather than used by the carrier.\n3. For an LED, plan a suitable series resistor; for a button, plan an input with a pull-up and a switch to ground.\n4. Share the carrier model and diagram with Sovra before choosing physical pins or runnable code.\n\nThere is no universal Compute Module socket map. No wiring numbers or runnable GPIO program are guessed here. Official starting point: {}",
            self.name,
            self.gpio_note(),
            self.gpio_source()
        )
    }

    pub fn troubleshooting(&self) -> String {
        let connection = if self.family == Family::Pico {
            "Use a data USB cable. BOOTSEL presents a storage drive, not a serial port. Check that MicroPython matches your exact Pico, choose its port in Thonny, and close other apps using it. USB detection does not verify a running program."
        } else {
            "For health checks, confirm power, network name, account, SSH access and Python 3. A host-key warning needs identity verification; do not bypass it. A GPIO program must run on the Pi with a compatible backend, not on this desktop."
        };
        format!(
            "# {} · Fix a connection or wiring problem\n\n1. {connection}\n2. Stop the project and disconnect power before inspecting wires. Check orientation, physical pin versus GPIO number, and a shared ground. Power pins are not GPIO outputs.\n3. An LED needs correct polarity and a series resistor. A button needs the switching pair of contacts and the intended pull-up. Remove old project wiring before trying another project.\n4. A pin reported as busy may be owned by another program, an enabled interface or an add-on board. Identify its owner rather than forcing access.\n5. {}\n\nAsk for help with the exact board/carrier, physical pins, parts and error message. Do not share passwords or private keys. Nothing is reset or reconfigured by this tool.",
            self.name,
            self.gpio_note()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raspberry_pi::{BOARDS, PinKind};

    #[test]
    fn projects_use_real_exposed_pins_and_do_not_guess_carriers() {
        for board in BOARDS {
            let led = board.led_project();
            let button = board.button_project();
            assert!(board.troubleshooting().contains("disconnect power"));
            if board.family == Family::ComputeModule {
                assert!(led.contains("carrier") && button.contains("carrier"));
                assert!(!led.contains("```python") && !button.contains("```python"));
                continue;
            }
            let (led_physical, led_gpio, button_physical, button_gpio) =
                if board.family == Family::Pico {
                    (20, 15, 19, 14)
                } else {
                    (11, 17, 11, 17)
                };
            for map in board.pin_maps().into_iter().filter(|map| map.id != "p5") {
                for (physical, gpio) in [(led_physical, led_gpio), (button_physical, button_gpio)] {
                    assert!(
                        map.rows
                            .iter()
                            .flatten()
                            .any(|pin| pin.physical == physical && pin.kind == PinKind::Gpio(gpio)),
                        "{}",
                        board.name
                    );
                }
            }
            assert!(led.contains("330 Ω") && led.contains("range(10)"));
            assert!(button.contains("30 seconds"));
            assert!(led.contains("```python") && button.contains("```python"));
        }
    }
}
