export type CatalogPart = {
  id: string;
  name: string;
  /** Markdown. Buy links inline as `[label](url)`; print files are listed from
   *  the parts index, so don't link them here. */
  description?: string;
  /** Model node names: parts-index instance ids (`<part>#<n>`, see
   *  tools/cad/index) or merged units (`ossm-pcb`). */
  instances: string[];
  parent?: string;
  offset?: [number, number, number];
  offsets?: Array<[number, number, number]>;
};

export const PART_CATALOG: CatalogPart[] = [
  {
    id: "main-motor-clamp",
    name: "Main motor clamp",
    description: `3D-printed motor mount that secures the 57AIM30 to the housing with embedded M5 lock nuts.`,
    parent: "motor-57aim30",
    offset: [0.095, 0, 0],
    instances: ["57aim-clamp#1"],
  },
  {
    id: "m5-lock-nut",
    name: "M5 lock nut",
    description: `M5 nylon-insert lock nut for motor fastening (vibration-resistant).

**Sources:**
- [Toolstation – M5 lock nuts](https://www.toolstation.nl/borgmoeren/p12051)`,
    parent: "main-motor-clamp",
    offset: [0.06, 0, 0],
    instances: ["nut-m5#1", "nut-m5#2", "nut-m5#3", "nut-m5#4"],
  },
  {
    id: "m3-nut-clamp",
    name: "M3 nut",
    description: `M3 captive nut used in the belt clamp.

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "main-motor-clamp",
    offsets: [
      [0, 0, -0.025],
      [0, 0, 0.025],
    ],
    instances: ["nut-m3#2", "nut-m3#1"],
  },
  {
    id: "motor-57aim30",
    name: "57AIM30 motor",
    description: `57AIM30 integrated NEMA23 closed-loop stepper-servo motor (~€100 incl. EU shipping).`,
    instances: ["motor#1"],
  },
  {
    id: "rail-mgn15h-400",
    name: "MGN15H 400mm rail",
    description: `MGN15H linear rail, 400 mm length (Hiwin preferred, AliExpress alternatives acceptable).`,
    parent: "belt-mechanics-rear",
    offset: [-0.135, 0, 0],
    instances: ["mgn15h-400mm-rail-part-1#1", "mgn15h-400mm-rail-part-2#1"],
  },
  {
    id: "motor-power-plug",
    name: "Motor power plug",
    description: `Phoenix-style screw terminal block for the motor's power leads. Typically included with the 57AIM30 motor.`,
    parent: "motor-57aim30",
    offset: [0, -0.03, 0],
    instances: ["degsonplug#1"],
  },
  {
    id: "mechanic-mount-top",
    name: "Mechanic mount top",
    description: `Upper half of the belt mechanics carriage that clamps onto the linear rail and houses bearings/dowels.`,
    parent: "rail-mgn15h-400",
    offset: [-0.08, 0, 0],
    instances: ["main-mount-top#1"],
  },
  {
    id: "belt",
    name: "Belt",
    description: `GT2 timing belt (9, 12, or 15 mm wide; length = rail length + ~10 cm).`,
    parent: "rail-mgn15h-400",
    offset: [-0.05, 0, 0],
    instances: ["gt2-belt#1"],
  },
  {
    id: "precision-dowel-m5",
    name: "Precision dowel (M5)",
    description: `Precision M5 stainless steel dowel pin kit covering all bearings and clamp pins.

**Sources:**
- [Amazon.nl – M5 dowel pin kit](https://www.amazon.nl/dp/B0DR7R87KQ)`,
    parent: "belt-mechanics-rear",
    offset: [-0.065, 0, 0],
    instances: ["precisiondowel#1"],
  },
  {
    id: "bearing-625",
    name: "625 bearing",
    description: `625-RS sealed ball bearing used as belt-guide bearings (6 required).

**Sources:**
- [Amazon.nl – 625-RS bearings](https://www.amazon.nl/dp/B071RXC7FV)`,
    parent: "precision-dowel-m5",
    offset: [-0.04, 0, 0],
    instances: [
      "bearing-625-2rs#1",
      "bearing-625-2rs#2",
      "bearing-625-2rs#3",
      "bearing-625-2rs#4",
      "bearing-625-2rs#5",
      "bearing-625-2rs#6",
    ],
  },
  {
    id: "top-cover",
    name: "Top cover",
    description: `Decorative cover over the PCB stack on the motor mount. Pick one of the solid covers, optionally paired with the transparent LED window insert.`,
    parent: "pcb-holder",
    offset: [0.09, 0, 0],
    instances: ["topcover#1", "ledcover#1"],
  },
  {
    id: "motor-cover-spacer",
    name: "Motor cover spacer",
    description: `Decorative spacer ring that sits on top of the motor mount and holds captive M3 nuts.`,
    parent: "main-motor-clamp",
    offset: [0.055, 0, 0],
    instances: ["motor-cover#1"],
  },
  {
    id: "belt-mechanics-rear",
    name: "Belt mechanics rear",
    description: `Rear half of the belt mechanics carriage that mates to the rail and front piece via dowel pins.`,
    parent: "motor-57aim30",
    offset: [-0.08, 0, 0],
    instances: ["main-mount-motor-side#1"],
  },
  {
    id: "pcb-holder",
    name: "PCB holder",
    description: `Mount that secures the main PCB (and optional brake chopper PCB) to the motor housing.`,
    parent: "motor-cover-spacer",
    offset: [0.035, 0, 0],
    instances: ["pcbholder#1"],
  },
  {
    id: "front-thread",
    name: "Front thread",
    description: `Front M24 toy-mount thread that screws onto the belt clamp for attaching accessories.`,
    parent: "rail-mgn15h-400",
    offset: [0, 0, 0.05],
    instances: ["thread#1"],
  },
  {
    id: "front-belt-tensioner",
    name: "Front belt tensioner",
    description: `Front belt tensioner block used to pinch and tension the GT2 belt at the carriage.`,
    parent: "front-clamp",
    offset: [0, 0, 0.025],
    instances: ["nut#1"],
  },
  {
    id: "front-clamp",
    name: "Front clamp",
    description: `Front belt clamp half that sandwiches the belt against the front thread.`,
    parent: "front-thread",
    offset: [0, 0, 0.065],
    instances: ["tightener#1"],
  },
  {
    id: "rear-tensioner-cover",
    name: "Rear tensioner cover",
    description: `Cover piece over the rear belt tensioner assembly.`,
    parent: "rear-bolt-m5x16",
    offset: [0, 0, -0.04],
    instances: ["cover#1"],
  },
  {
    id: "back-belt-tensioner",
    name: "Back belt tensioner",
    description: `Rear belt tensioner block, identical print to the front tensioner, used at the back of the belt loop.`,
    parent: "rear-nut-clamp",
    offset: [0, 0, -0.045],
    instances: ["nut#2"],
  },
  {
    id: "rear-thread",
    name: "Rear thread",
    description: `Rear-side clamp body that pairs with the front thread to form the M24 mount/belt clamp.`,
    parent: "rail-mgn15h-400",
    offset: [0, 0, -0.06],
    instances: ["rear-mount-thread#1"],
  },
  {
    id: "rear-nut-clamp",
    name: "Rear nut clamp",
    description: `Printed captive nut clamp used at the rear of the belt clamp assembly.`,
    parent: "rear-thread",
    offset: [0, 0, -0.06],
    instances: ["tightener#2"],
  },
  {
    id: "motor-pulley",
    name: "Motor pulley (GT2 20T)",
    description: `GT2 20-tooth pulley sized for up to 15 mm belt width.`,
    parent: "motor-57aim30",
    offset: [-0.035, 0, 0],
    instances: ["gt2-20t-sh8#1"],
  },
  {
    id: "rear-bolt-m5x16",
    name: "Rear bolt (M5×16)",
    description: `M5x16 socket-head cap bolt (from M5 bolt kit).

**Sources:**
- [Amazon.nl – M5 nut & bolt kit](https://www.amazon.nl/dp/B0DPWYNDZF)`,
    parent: "back-belt-tensioner",
    offset: [0, 0, -0.03],
    instances: ["shcs-m5x16#1"],
  },
  {
    id: "back-belt-tensioner-nut",
    name: "Back belt tensioner nut",
    description: `M5 captive nut for the rear belt tensioner.

**Sources:**
- [Amazon.nl – M5 nut & bolt kit](https://www.amazon.nl/dp/B0DPWYNDZF)`,
    parent: "rear-thread",
    offset: [0, 0, 0.045],
    instances: ["nut-m5#8"],
  },
  {
    id: "front-bolt-m3x16",
    name: "Front bolt (M3×16)",
    description: `M3x16 socket-head cap bolt for the optional front belt clamp fastening.

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "front-thread",
    offset: [0, -0.035, 0],
    instances: ["shcs-m3x16#1"],
  },
  {
    id: "front-belt-tensioner-nut",
    name: "Front belt tensioner nut",
    description: `M3 captive nut for the front belt tensioner clamp.

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "front-thread",
    offset: [0, 0.02, 0],
    instances: ["nut-m3#6"],
  },
  {
    id: "carriage-m3-bolts-rail",
    name: "Carriage M3×0.5 bolts (rail side)",
    description: `M3x12 socket-head bolts attaching the carriage to the MGN15H rail block (4 pcs).

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "mechanic-mount-top",
    offset: [0, 0.025, 0],
    instances: ["shcs-m3x12-mirrored#1", "shcs-m3x12-mirrored#2"],
  },
  {
    id: "carriage-m3-bolts-motor",
    name: "Carriage M3×0.5 bolts (motor side)",
    description: `M3 bolts joining the belt-mechanics carriage halves (from M3 kit).

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "belt-mechanics-rear",
    offset: [0, 0.05, 0],
    instances: ["shcs-m3x12#1", "shcs-m3x12#2"],
  },
  {
    id: "pcb-bolts",
    name: "PCB M3 bolts",
    description: `M3x8 socket-head bolts mounting the PCB to the printed holder (2 pcs).

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "main-pcb",
    offset: [0.045, 0, 0],
    instances: ["shcs-m3x8#1", "shcs-m3x8#2"],
  },
  {
    id: "pcb-nuts",
    name: "PCB M3 nuts",
    description: `M3 captive nuts in the PCB holder.

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "pcb-holder",
    offset: [0.05, 0, 0],
    instances: ["nut-m3#4", "nut-m3#5"],
  },
  {
    id: "rail-case-nuts",
    name: "Rail case nuts",
    description: `M5 captive nuts seated in the motor housing for rail/case bolts.

**Sources:**
- [Amazon.nl – M5 nut & bolt kit](https://www.amazon.nl/dp/B0DPWYNDZF)`,
    parent: "belt-mechanics-rear",
    offsets: [
      [0, 0.025, 0],
      [0, 0.025, 0],
      [0.025, 0, 0],
    ],
    instances: ["nut-m5#6", "nut-m5#7", "nut-m5#5"],
  },
  {
    id: "rail-case-bolts",
    name: "Rail case bolts",
    description: `M5x30 socket-head bolts fastening the motor housing/case to the rail end.

**Sources:**
- [Amazon.nl – M5 nut & bolt kit](https://www.amazon.nl/dp/B0DPWYNDZF)`,
    parent: "mechanic-mount-top",
    offset: [-0.05, 0, 0],
    instances: ["shcs-m5x30#6", "shcs-m5x30#5", "shcs-m5x20#1"],
  },
  {
    id: "clamp-bolts",
    name: "Clamp bolts",
    description: `M5x30 (x2) and M5x20 (x1) socket-head bolts for the front mount/belt clamp.

**Sources:**
- [Amazon.nl – M5 nut & bolt kit](https://www.amazon.nl/dp/B0DPWYNDZF)`,
    parent: "belt-mechanics-rear",
    offset: [-0.05, 0, 0],
    instances: ["shcs-m5x30#1", "shcs-m5x30#2", "shcs-m5x30#3", "shcs-m5x30#4"],
  },
  {
    id: "pcb-holder-bolts",
    name: "PCB holder bolts",
    description: `M3x40 bolts that fasten the PCB holder through the housing (2 pcs).

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "pcb-holder",
    offset: [0.05, 0, 0],
    instances: ["shcs-m3x40#1", "shcs-m3x40#2"],
  },
  {
    id: "motor-cover-spacer-nut",
    name: "Motor cover spacer nut",
    description: `M3 captive nut pressed into the motor cover spacer ring.

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "motor-cover-spacer",
    offset: [0, -0.05, 0],
    instances: ["nut-m3#3"],
  },
  {
    id: "top-cover-bolt",
    name: "Top cover bolt",
    description: `Single M3x12 socket-head bolt securing the top cover.

**Sources:**
- [Amazon.nl – M3 nut & bolt kit](https://www.amazon.nl/dp/B0B3MGZ7T2)`,
    parent: "top-cover",
    offset: [0.045, 0, 0],
    instances: ["shcs-m3x12#5"],
  },
  {
    id: "main-pcb",
    name: "Main PCB",
    description: `Custom 28V USB-C PD ESP32-S3 controller PCB with RS485 and 140W power delivery.

**Design:**
- [PCB design on GitHub](https://github.com/jollydodo/OSSM-ALT-Edition/tree/main/PCB-Design)`,
    parent: "pcb-holder",
    offset: [0.05, 0, 0],
    instances: ["ossm-pcb"],
  },
];
