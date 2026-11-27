//! Connector facts, shared by the native pin map and the MCP guide.
//! Sources and revision caveats are recorded in docs/raspberry-pi.md.

use super::{Board, Family};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinKind {
    Gpio(u8),
    Power(&'static str),
    Ground,
    Reserved(u8),
    Control(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub physical: u8,
    pub kind: PinKind,
}

impl Pin {
    pub fn label(self) -> String {
        match self.kind {
            PinKind::Gpio(number) => format!("GPIO {number}"),
            PinKind::Power(name) => format!("{name} power"),
            PinKind::Control(name) => name.into(),
            PinKind::Ground => "Ground".into(),
            PinKind::Reserved(number) => format!("ID / GPIO {number}"),
        }
    }

    pub fn description(self) -> String {
        let detail = match self.kind {
            PinKind::Gpio(number) => format!("Digital input/output. Use {number} in code, not {}. Use 3.3 V signals; do not connect 5 V.", self.physical),
            PinKind::Power("VBUS") => "USB supply connection, nominally 5 V, not a programmable GPIO. Do not connect it to a GPIO signal.".into(),
            PinKind::Power("VSYS") => "System power input, not a programmable GPIO. Follow the Pico power-supply guide before using an external supply.".into(),
            PinKind::Power("3V3 OUT") => "Regulated 3.3 V output, not a programmable GPIO. Check the board's supply limits before powering other parts.".into(),
            PinKind::Power(_) => "Power connection, not a programmable GPIO. Follow this board's power instructions; do not treat it as a signal pin.".into(),
            PinKind::Ground => "Ground is the shared return path for your circuit, not a programmable GPIO.".into(),
            PinKind::Reserved(_) => "Reserved for add-on board identification. Leave this out of beginner projects.".into(),
            PinKind::Control(_) => "Board control or analogue reference, not a general-purpose output. Leave this out of beginner projects.".into(),
        };
        format!(
            "Physical pin {} · {}. {detail}",
            self.physical,
            self.label()
        )
    }
}

#[derive(Debug, Clone)]
pub struct PinMap {
    pub id: &'static str,
    pub title: &'static str,
    pub orientation: &'static str,
    pub rows: Vec<[Pin; 2]>,
}

use PinKind::{Control as C, Gpio as G, Ground as N, Power as P, Reserved as R};

const HEADER: [PinKind; 40] = [
    P("3.3 V"),
    P("5 V"),
    G(2),
    P("5 V"),
    G(3),
    N,
    G(4),
    G(14),
    N,
    G(15),
    G(17),
    G(18),
    G(27),
    N,
    G(22),
    G(23),
    P("3.3 V"),
    G(24),
    G(10),
    N,
    G(9),
    G(25),
    G(11),
    G(8),
    N,
    G(7),
    R(0),
    R(1),
    G(5),
    N,
    G(6),
    G(12),
    G(13),
    N,
    G(19),
    G(16),
    G(26),
    G(20),
    N,
    G(21),
];
const PICO: [PinKind; 40] = [
    G(0),
    G(1),
    N,
    G(2),
    G(3),
    G(4),
    G(5),
    N,
    G(6),
    G(7),
    G(8),
    G(9),
    N,
    G(10),
    G(11),
    G(12),
    G(13),
    N,
    G(14),
    G(15),
    G(16),
    G(17),
    N,
    G(18),
    G(19),
    G(20),
    G(21),
    N,
    G(22),
    C("RUN"),
    G(26),
    G(27),
    N,
    G(28),
    C("ADC_VREF"),
    P("3V3 OUT"),
    C("3V3_EN"),
    N,
    P("VSYS"),
    P("VBUS"),
];

fn header(id: &'static str, title: &'static str, count: u8, revision_one: bool) -> PinMap {
    let mut pins = HEADER;
    if revision_one {
        pins[2] = G(0);
        pins[4] = G(1);
        pins[12] = G(21);
    }
    PinMap {
        id,
        title,
        orientation: "Numbered connector view, not a picture of your board. Locate the board's pin 1 marking before wiring; odd pins are on one row, even pins on the other.",
        rows: (1..=count)
            .step_by(2)
            .map(|physical| {
                [
                    Pin {
                        physical,
                        kind: pins[usize::from(physical - 1)],
                    },
                    Pin {
                        physical: physical + 1,
                        kind: pins[usize::from(physical)],
                    },
                ]
            })
            .collect(),
    }
}

fn legacy_extra() -> PinMap {
    let pins = [P("5 V"), P("3.3 V"), G(28), G(29), G(30), G(31), N, N];
    PinMap {
        id: "p5",
        title: "P5 auxiliary pads · revision 2 only",
        orientation: "A separate, usually unpopulated 8-pin connector, not an extension of P1. Confirm P5 pin 1 from the board schematic; beginners should use P1.",
        rows: (1_u8..=8)
            .step_by(2)
            .map(|physical| {
                [
                    Pin {
                        physical,
                        kind: pins[usize::from(physical - 1)],
                    },
                    Pin {
                        physical: physical + 1,
                        kind: pins[usize::from(physical)],
                    },
                ]
            })
            .collect(),
    }
}

impl Board {
    pub fn pin_maps(&self) -> Vec<PinMap> {
        match self.family {
            Family::ComputeModule => Vec::new(),
            Family::Pico => vec![PinMap {
                id: "pico",
                title: "Pico edge pins · all Pico and Pico 2 variants",
                orientation: "Top/component view with USB at the top: pin 1 is upper left, pin 40 upper right. GPIO 26–28 are also analogue inputs; pin 33 is analogue ground. Debug pads are separate.",
                rows: (1_u8..=20)
                    .map(|physical| {
                        [
                            Pin {
                                physical,
                                kind: PICO[usize::from(physical - 1)],
                            },
                            Pin {
                                physical: 41 - physical,
                                kind: PICO[usize::from(40 - physical)],
                            },
                        ]
                    })
                    .collect(),
            }],
            Family::Computer if self.id == "pi-1-b" => vec![
                header("p1-rev1", "P1 · Model B revision 1", 26, true),
                header("p1-rev2", "P1 · Model B revision 2", 26, false),
                legacy_extra(),
            ],
            Family::Computer if self.id == "pi-1-a" => vec![
                header("p1-rev2", "P1 · original Model A", 26, false),
                legacy_extra(),
            ],
            Family::Computer | Family::Zero | Family::Keyboard => {
                vec![header("j8", "40-pin GPIO connector", 40, false)]
            }
        }
    }

    pub fn gpio_note(&self) -> &'static str {
        match self.family {
            Family::ComputeModule => {
                "GPIO is brought out through the module's connectors to a carrier board. The module name alone cannot identify the carrier's socket positions or voltage configuration. Read the exact carrier's schematic and pin labels before wiring. CM4IO and CM5IO have a 40-pin header; CMIO/CMIO3 and custom carriers have different layouts. A standard Pi map must not be assumed."
            }
            Family::Pico => {
                "There are 40 edge positions, including 26 GPIOs. GPIO 23–25 and 29 are not external edge pins. Wireless boards drive their built-in light differently; use Pin('LED') for that light. Boards without fitted headers need suitable connections before breadboard projects."
            }
            Family::Keyboard => {
                "The GPIO connector is at the back of the keyboard and rotated relative to classic boards. Use the printed pin 1 and pin 40 markings, not the screen's left/right orientation."
            }
            Family::Zero => {
                "The connector follows the 40-pin Pi layout. Some Zero versions have holes without fitted pins; check your actual board before planning a breadboard project."
            }
            Family::Computer if self.id == "pi-1-b" => {
                "Check the original Model B's board revision first. Physical pins 3, 5 and 13 change between revision 1 and revision 2. Both P1 maps are shown; P5 exists only on revision 2. Do not choose a revision from memory capacity alone."
            }
            Family::Computer if self.id == "pi-1-a" => {
                "This original Model A has a 26-pin P1 connector and separate P5 pads. It is not the 40-pin Model A+."
            }
            Family::Computer => {
                "This is the standard 40-pin connector. Physical pin 11 is GPIO 17 in code. Pins 27 and 28 are reserved for add-on board identification. I2C uses GPIO 2/3, the usual UART uses 14/15, and SPI0 uses 7–11; enabled functions and attached hardware can already own these pins."
            }
        }
    }

    pub const fn gpio_source(&self) -> &'static str {
        match self.family {
            Family::Pico => {
                "https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html#board-layout"
            }
            Family::ComputeModule => {
                "https://www.raspberrypi.com/documentation/computers/compute-module.html"
            }
            Family::Computer | Family::Zero | Family::Keyboard => {
                "https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#gpio"
            }
        }
    }

    pub fn gpio_guide(&self) -> String {
        let mut text = format!(
            "# {} · GPIO pins\n\nReference map, not live pin readings.\n\n{}\n\nPower off before changing wires. Use 3.3 V GPIO signals; never connect 5 V or a motor directly to a GPIO. An LED needs a series resistor.\n",
            self.name,
            self.gpio_note()
        );
        for map in self.pin_maps() {
            let _ = write!(
                text,
                "\n## {}\n\n{}\n\n| Physical pin | Function | Physical pin | Function |\n| --- | --- | --- | --- |\n",
                map.title, map.orientation
            );
            for [left, right] in map.rows {
                let _ = writeln!(
                    text,
                    "| {} | {} | {} | {} |",
                    left.physical,
                    left.label(),
                    right.physical,
                    right.label()
                );
            }
        }
        let _ = write!(
            text,
            "\nOfficial board guide: {}\nLegacy P1/P5 mapping: https://github.com/gpiozero/gpiozero/blob/master/gpiozero/pins/data.py\n\nIncluded tools: setup guide, pin map, first project, LED project, button project, troubleshooting and device checks. Projects are instructions and code to review and run on your own board; opening them does not change a pin.",
            self.gpio_source()
        );
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raspberry_pi::{BOARDS, board};

    fn pin(id: &str, map: usize, physical: u8) -> PinKind {
        board(id).expect("board").pin_maps()[map]
            .rows
            .iter()
            .flatten()
            .find(|pin| pin.physical == physical)
            .expect("pin")
            .kind
    }

    #[test]
    fn pin_maps_cover_the_connectors_without_inventing_carrier_wiring() {
        for board in BOARDS {
            let maps = board.pin_maps();
            assert_eq!(maps.is_empty(), board.family == Family::ComputeModule);
            for map in maps {
                let numbers: std::collections::BTreeSet<_> =
                    map.rows.iter().flatten().map(|pin| pin.physical).collect();
                assert_eq!(numbers.len(), map.rows.len() * 2);
                assert_eq!(
                    numbers.iter().copied().collect::<Vec<_>>(),
                    (1..=u8::try_from(numbers.len()).expect("small header")).collect::<Vec<_>>()
                );
                for pin in map.rows.iter().flatten() {
                    assert_ne!(pin.label(), "");
                    assert!(
                        pin.description()
                            .contains(&format!("Physical pin {}", pin.physical))
                    );
                }
            }
            assert!(
                board
                    .gpio_source()
                    .starts_with("https://www.raspberrypi.com/")
            );
            assert!(board.gpio_guide().contains("not live"));
        }
        assert_eq!(pin("pi-5", 0, 11), G(17));
        assert_eq!(pin("pi-5", 0, 2), P("5 V"));
        assert_eq!(pin("pi-5", 0, 27), R(0));
        assert_eq!(pin("pi-1-b", 0, 13), G(21));
        assert_eq!(pin("pi-1-b", 1, 13), G(27));
        assert_eq!(pin("pi-1-b", 0, 3), G(0));
        assert_eq!(pin("pi-1-b", 1, 3), G(2));
        assert_eq!(pin("pi-1-a", 0, 26), G(7));
        assert_eq!(pin("pi-1-a", 1, 3), G(28));
        for id in ["pico", "pico-w", "pico-2", "pico-2-w-headers"] {
            assert_eq!(pin(id, 0, 11), G(8));
            assert_eq!(pin(id, 0, 34), G(28));
            assert_eq!(pin(id, 0, 40), P("VBUS"));
            let map = board(id).expect("pico").pin_maps().remove(0);
            assert_eq!(map.rows[0][1].physical, 40);
            assert_eq!(
                map.rows
                    .iter()
                    .flatten()
                    .filter(|pin| matches!(pin.kind, G(_)))
                    .count(),
                26
            );
        }
        assert!(
            board("pi-500")
                .expect("keyboard")
                .gpio_note()
                .contains("rotated")
        );
    }
}
