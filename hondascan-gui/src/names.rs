//! Plain-English names for the signals people see most, keyed by HDS symbol (ported from
//! hondascan-app/src/lib/names.ts). HDS's own descriptions stay available as `raw`. A symbol that is not here keeps its
//! tidied HDS description. Meanings are the standard ones for the part; where a name is a judgement ("shift status")
//! the blurb stays generic rather than guess.
use crate::api::SignalDef;
use crate::format::tidy;

const NAMES: &[(&str, &str, &str)] = &[
    ("NE", "Engine speed", "How fast the crankshaft turns, in revolutions per minute (rpm)."),
    ("NEW", "Engine speed", "How fast the crankshaft turns, in revolutions per minute (rpm)."),
    ("VSP", "Vehicle speed", "Road speed as the computer works it out from the wheel and shaft sensors."),
    ("ECT", "Coolant temperature", "Engine coolant temperature. A warm engine sits around 80–100 °C (176–212 °F)."),
    ("TW", "Coolant temperature", "Engine coolant temperature. A warm engine sits around 80–100 °C (176–212 °F)."),
    ("ECT2A", "Coolant temperature (sensor 2)", "A second coolant temperature sensor."),
    ("IAT", "Intake air temperature", "Temperature of the air going into the engine."),
    ("TA", "Intake air temperature", "Temperature of the air going into the engine."),
    ("SIAT", "Intake air temperature (sensor 2)", "A second intake air temperature sensor."),
    ("MAP", "Intake pressure", "Air pressure inside the intake manifold (MAP). Low at idle, close to outside air at full throttle."),
    ("PB", "Intake pressure", "Air pressure inside the intake manifold (MAP). Low at idle, close to outside air at full throttle."),
    ("PBAENAN", "Intake pressure (fine)", "Intake manifold pressure at higher resolution."),
    ("BARO", "Outside air pressure", "Barometric pressure; drops with altitude and weather."),
    ("PA", "Outside air pressure", "Barometric pressure; drops with altitude and weather."),
    ("APP", "Accelerator pedal", "How far the accelerator pedal is pressed."),
    ("APSAT", "Accelerator pedal", "How far the accelerator pedal is pressed."),
    ("ABSTH", "Throttle opening", "How far the throttle valve is open."),
    ("RELTH", "Throttle opening (relative)", "Throttle valve opening relative to its closed position."),
    ("FLEVELF", "Fuel level", "How full the fuel tank is."),
    ("VB", "Battery voltage", "The 12 V battery as the computer sees it. About 12.5–14.5 V with the engine running."),
    ("VBENAN", "Battery voltage (fine)", "Battery voltage at higher resolution."),
    ("VKAM", "Memory backup voltage", "Voltage that keeps the computer’s memory alive when the car is off."),
    ("GAIR", "Air flow", "Air entering the engine, in grams per second (MAF sensor)."),
    ("AFM", "Air flow sensor", "The mass air flow (MAF) sensor’s signal."),
    ("IG", "Spark timing", "How many degrees before top dead centre the spark fires."),
    ("IGENAN", "Spark timing (fine)", "Spark timing at higher resolution."),
    ("TOUT", "Injector pulse", "How long each fuel injection lasts, in milliseconds. Longer means more fuel."),
    ("ACC", "A/C compressor clutch", "Whether the air-conditioning compressor is engaged."),
    ("ACS", "A/C switch", "Whether the A/C is switched on."),
    ("ACGF", "Alternator load", "How hard the alternator is working."),
    ("WARN", "Check-engine light", "Whether the engine warning light (MIL) is on."),
    ("BKSW", "Brake pedal switch", "Whether the brake pedal is pressed."),
    ("PBRK1GN", "Brake fluid pressure", "Pressure in the brake lines, from sensor A."),
    ("PBRK2GN", "Brake fluid pressure (sensor B)", "Pressure in the brake lines, from sensor B."),
    ("TATF", "Transmission fluid temperature", "CVT fluid temperature. Normal driving is roughly 70–100 °C (158–212 °F)."),
    ("NDR", "Drive pulley speed", "Speed of the CVT’s input pulley, the one the engine turns."),
    ("NDN", "Driven pulley speed", "Speed of the CVT’s output pulley, the one that sends power on to the wheels."),
    ("PDN", "Driven pulley pressure", "Hydraulic pressure that squeezes the output pulley’s belt."),
    ("RATIO", "Pulley ratio", "Current CVT ratio: input speed compared with output speed."),
    ("ESCP", "Start clutch slip", "How much the start clutch is slipping, as when pulling away."),
    ("SLOPE", "Road slope", "The incline the car is on, from the transmission’s tilt sensor."),
    ("LONACAD", "Slope sensor voltage", "Raw signal of the tilt sensor."),
    ("SFTMAPST", "Shift map (raw code)", "Which shift schedule the transmission computer is using (Honda: “Shift Map Number”). Honda lists no meaning for each value, so HDS shows the bare number. On this Accord it reads 0x3000 stopped and changes with how you drive."),
    ("NT", "Turbine speed", "Speed of the torque converter’s output side."),
    ("NVABS", "Wheel speed (rpm)", "Wheel rotation speed from the ABS sensors, in rpm."),
    ("VLVABS", "Wheel speed", "Wheel speed from the ABS sensors."),
    ("VLVABSFL", "Front left wheel speed", "From the ABS wheel sensor, relayed by the transmission computer."),
    ("VLVABSFR", "Front right wheel speed", "From the ABS wheel sensor, relayed by the transmission computer."),
    ("VLVABSRL", "Rear left wheel speed", "From the ABS wheel sensor, relayed by the transmission computer."),
    ("VLVABSRR", "Rear right wheel speed", "From the ABS wheel sensor, relayed by the transmission computer."),
    ("VLNDN", "Driven pulley speed (as road speed)", "Output pulley speed expressed as km/h."),
    ("NVEL1", "Secondary shaft speed 1", "Speed of the transmission’s secondary shaft (rpm)."),
    ("NVEL2", "Secondary shaft speed 2", "Speed of the transmission’s secondary shaft (rpm)."),
    ("VLVEL1", "Secondary shaft speed", "Secondary shaft speed expressed as km/h."),
    ("VLVEL2", "Secondary shaft speed 2", "Secondary shaft speed expressed as km/h."),
    ("NEOP", "Electric oil pump speed", "Speed of the transmission’s electric oil pump."),
    ("NEOPCMD", "Electric oil pump target", "The speed the computer is asking the oil pump for."),
    ("CVTWUPST", "Transmission warm-up", "Whether the transmission is still warming up."),
    ("CLPMOD", "Creep control", "Status of the control that stops the car creeping forward at rest."),
    ("SHA", "Shift solenoid A", "An electrically operated valve that directs fluid for shifting."),
    ("SHB", "Shift solenoid B", "An electrically operated valve that directs fluid for shifting."),
    ("SHO/P", "Shift solenoid O/P", "An electrically operated valve that directs fluid for shifting."),
    ("SLS", "Shift-lock solenoid", "Locks the shifter in Park unless the brake is pressed."),
    ("SDN", "Downshift switch", "The downshift paddle or button."),
    ("SUP", "Upshift switch", "The upshift paddle or button."),
];

const PLAIN_VOLTS: [&str; 3] = ["VB", "VBENAN", "VKAM"]; // volts that are the thing itself, not a sensor signal

pub struct Named { pub name: String, pub about: Option<String>, pub raw: String }

/// What to call a signal: a plain name, a one-line meaning when known, and HDS's own wording as `raw`.
pub fn friendly(def: &SignalDef) -> Named {
    let mut raw = vec![def.description.as_str(), def.symbol.as_str()];
    raw.dedup();
    let raw = raw.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    match NAMES.iter().find(|(k, _, _)| *k == def.symbol) {
        None => Named { name: if def.description.is_empty() { def.symbol.clone() } else { tidy(&def.description) }, about: None, raw },
        Some((_, name, about)) => {
            let volts = def.unit == "V" && !PLAIN_VOLTS.contains(&def.symbol.as_str());
            if volts { Named { name: format!("{name} (sensor voltage)"), about: Some("The raw voltage the computer reads from this sensor.".into()), raw } }
            else { Named { name: name.to_string(), about: Some(about.to_string()), raw } }
        }
    }
}

// Shift map (SFTMAPST): Honda gives no table, so these are what the word was while driving the 2015 Accord
// (2026-10-12, 1421 reads): the commonest values and what the car was doing. Anything else is said not to be known.
const SHIFT_MAP: &[(u32, &str)] = &[
    (0x3000, "Stopped, idling"), (0x7000, "Low speed, foot off the pedal"), (0x2000, "Foot off the pedal"), (0x2900, "Accelerating"),
    (0x7800, "Accelerating"), (0x2010, "Coasting"), (0x2940, "Hard acceleration"), (0x7850, "Hard acceleration"),
];

/// A plain-words reading of a raw value, for the few signals Honda leaves as a bare number.
pub fn explain(symbol: &str, value: Option<f64>) -> Option<String> {
    let v = value? as u32;
    if symbol != "SFTMAPST" { return None; }
    Some(match SHIFT_MAP.iter().find(|(k, _)| *k == v) {
        Some((_, w)) => format!("{w} · 0x{v:04X}"),
        None => format!("0x{v:04X} · not seen on a known drive"),
    })
}
