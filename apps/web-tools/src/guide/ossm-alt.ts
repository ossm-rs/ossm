// Step-by-step build guide for the OSSM ALT (Heavy, MGN15H).
//
// Each step adds model nodes (parts-index instance ids, see tools/cad/index)
// to what earlier steps built; the guide view shows earlier parts in place and
// slides this step's parts in. Every node in the model should appear in
// exactly one step's `instances`.
//
// Sub-assemblies: a step with `stage` builds its parts off to one side (at
// that offset from their assembled position); a later step lists them in
// `place` to bring them onto the build.
//
// DRAFT: the order follows the geometry (what holds what) and the text uses
// upstream's written notes; the full procedure is in upstream's videos and
// still needs a walkthrough.

export type GuideStep = {
  id: string;
  title: string;
  /** Markdown. */
  body: string;
  /** Model nodes this step adds. */
  instances: string[];
  /** The step's main part: the camera centres on it and it never moves (if
   *  this step adds it, it appears in place; the rest travel to it). */
  main: string;
  /** Where this step's parts slide in from (metres, model frame), overriding
   *  the diagram catalog's explode offsets for the guide. */
  from?: Record<string, [number, number, number]>;
  /** Build this step's parts at this offset (metres, model frame) until a
   *  later step places them. */
  stage?: [number, number, number];
  /** Staged parts from earlier steps that this step moves into position. */
  place?: string[];
  /** How the belt lies from this step on (carried forward). `feed` [from, to]
   *  slides a hanging belt's tip in during the step (0: at the clamp exit,
   *  1: where it ends up). */
  belt?:
    | { mode: "routed" }
    | {
        mode: "hang";
        end: "front";
        feed: [number, number];
        /** How far back along the hanging belt (metres) the tip starts. */
        approach?: number;
      }
    /** The free tip carried to the idlers and fed through (see model/belt.ts). */
    | { mode: "thread" }
    /** The tip fed into the rear clamp, leaving `slack` metres sagging behind it. */
    | { mode: "rear"; feed: [number, number]; slack: [number, number] };
  /** Seconds for the slide phase (default 0.9). */
  slide?: number;
  /** Frame the whole device rather than the main part and this step's parts. */
  frame?: "all";
  /** Where the camera looks from (a direction, model frame: +X is the PCB
   *  end, +Y up, +Z towards the toy end). Carries forward to later steps until
   *  one sets another. */
  view?: [number, number, number];
};

export type GuideSection = {
  title: string;
  steps: GuideStep[];
};

export const GUIDE_INTRO = `Draft guide, built from the model and upstream's notes. The full procedure is in
upstream's assembly videos: [part 1](https://youtu.be/EqzNXQzpHu0), [part 2](https://youtu.be/FnY4Sz64H2A).`;

export const GUIDE: GuideSection[] = [
  {
    title: "Motor and motor clamp",
    steps: [
      {
        id: "motor-mount-m5-nuts",
        title: "Fit the M5 nuts into the main motor clamp",
        main: "57aim-clamp#1",
        body: `Press an M5 nut into each of the four hex pockets. Upstream recommends M5 **lock nuts**: they take some force to fit but won't loosen with vibration (plain nuts fit more easily but can work loose).`,
        instances: ["57aim-clamp#1", "nut-m5#1", "nut-m5#2", "nut-m5#3", "nut-m5#4"],
        view: [0.58, 0.58, 0.58],
      },
      {
        id: "motor-mount-m3-nuts",
        title: "Fit the M3 nuts into the main motor clamp",
        main: "57aim-clamp#1",
        body: `Press an M3 nut into each of the two side slots: halfway by hand, then push it home with a screwdriver. The fit is deliberately tight so the nuts don't fall out during assembly.`,
        instances: ["nut-m3#1", "nut-m3#2"],
      },
      {
        id: "rear-plate-m5-nuts",
        title: "Fit the M5 nuts into the belt mechanics rear",
        main: "main-mount-motor-side#1",
        body: `Press an M5 nut into the hex pocket at the bottom of the belt mechanics rear, and one into each of the two pockets at the top.`,
        instances: ["main-mount-motor-side#1", "nut-m5#5", "nut-m5#6", "nut-m5#7"],
        stage: [-0.12, 0, 0],
      },
      {
        id: "motor",
        title: "Seat the motor in the main motor clamp",
        main: "57aim-clamp#1",
        view: [-0.58, 0.58, 0.58],
        body: `Slide the 57AIM30 into the main motor clamp from the belt end, back first, so the shaft points out towards the belt end.

Mind the orientation: the side of the clamp marked **RS485** lines up with the white RS485 connector on the motor and faces the top of the machine.`,
        instances: ["motor#1"],
        // Shaft first, from the belt end.
        from: { "motor#1": [-0.09, 0, 0] },
      },
      {
        id: "rear-plate-on-motor",
        title: "Fit the belt mechanics rear onto the motor",
        main: "motor#1",
        body: `Fit the belt mechanics rear onto the front of the motor and secure it with four M5×30: through the rear plate and the motor flange into the main motor clamp's M5 nuts.`,
        instances: ["shcs-m5x30#1", "shcs-m5x30#2", "shcs-m5x30#3", "shcs-m5x30#4"],
        place: ["main-mount-motor-side#1", "nut-m5#5", "nut-m5#6", "nut-m5#7"],
        // Start behind the housing, so they follow it in rather than pass through it.
        from: {
          "shcs-m5x30#1": [-0.2, 0, 0],
          "shcs-m5x30#2": [-0.2, 0, 0],
          "shcs-m5x30#3": [-0.2, 0, 0],
          "shcs-m5x30#4": [-0.2, 0, 0],
        },
      },
    ],
  },
  {
    title: "Belt mechanics",
    steps: [
      {
        id: "bearings",
        title: "Slide the bearings onto the dowel pins",
        main: "precisiondowel#1",
        body: `Slide three 625 bearings onto each of the two precision dowel pins.`,
        instances: [
          "precisiondowel#1",
          "bearing-625-2rs#1",
          "bearing-625-2rs#2",
          "bearing-625-2rs#3",
          "bearing-625-2rs#4",
          "bearing-625-2rs#5",
          "bearing-625-2rs#6",
        ],
        stage: [-0.1, 0, 0],
      },
      {
        id: "bearings-in",
        title: "Press the bearing stacks into the belt mechanics rear",
        main: "main-mount-motor-side#1",
        body: `Press the dowel pins, with their bearings, into the belt mechanics rear. There should be no play: expect to press or tap them in firmly.`,
        instances: [],
        place: [
          "precisiondowel#1",
          "bearing-625-2rs#1",
          "bearing-625-2rs#2",
          "bearing-625-2rs#3",
          "bearing-625-2rs#4",
          "bearing-625-2rs#5",
          "bearing-625-2rs#6",
        ],
      },
      {
        id: "pulley",
        title: "Fit the pulley to the motor shaft",
        main: "motor#1",
        body: `Slide the GT2 20T pulley onto the motor shaft and tighten the grub screw firmly: a loose grub screw lets the belt slip.`,
        instances: ["gt2-20t-sh8#1"],
      },
      {
        id: "rail",
        title: "Fit the rail",
        main: "main-mount-motor-side#1",
        body: `Fit the MGN15H rail, with its carriage, onto the belt mechanics rear.`,
        instances: ["mgn15h-400mm-rail-part-1#1", "mgn15h-400mm-rail-part-2#1"],
      },
      {
        id: "rail-bolts",
        title: "Bolt the rail carriage to the housing",
        main: "mgn15h-400mm-rail-part-2#1",
        body: `Secure the carriage with two M3×12.`,
        instances: ["shcs-m3x12#1", "shcs-m3x12#2"],
      },
      {
        id: "toy-mount-bolt",
        title: "Fit the M3 nut and bolt into the toy mount",
        main: "thread#1",
        body: `Start the front belt clamp, the toy mount: insert the M3 nut onto the M3×16 bolt.`,
        instances: ["thread#1", "shcs-m3x16#1", "nut-m3#6"],
      },
      {
        id: "toy-mount-belt",
        title: "Pass the belt through the end stop",
        main: "thread#1",
        slide: 1.8, // the belt moves at half speed
        body: `Pass the front end of the GT2 belt through the end stop. You need the rail length plus about 10 cm of belt; up to 15 mm wide fits.`,
        instances: ["gt2-belt#1"],
        // Fed in through the slot and out of the bottom; the rest hangs off the clamp.
        belt: { mode: "hang", end: "front", feed: [0, 1], approach: 0.08 },
      },
      {
        id: "toy-mount-tightener",
        title: "Add the tightening nut",
        main: "thread#1",
        body: `Add the tightening nut.`,
        instances: ["tightener#1"],
      },
      {
        id: "toy-mount-end-nut",
        title: "Screw on the end nut",
        main: "thread#1",
        body: `Screw the end nut onto the thread.`,
        instances: ["nut#1"],
      },
      {
        id: "toy-mount-thread-belt",
        title: "Thread the belt through the toy mount",
        main: "thread#1",
        body: `Thread the belt through the toy mount's 3D printed screw.`,
        instances: [],
      },
      {
        id: "back-stop-nut",
        title: "Fit the back belt tensioner nut into the rear thread",
        main: "rear-mount-thread#1",
        body: `Press the M5 nut (the back belt tensioner nut) into the rear thread.`,
        instances: ["rear-mount-thread#1", "nut-m5#8"],
        stage: [0, 0, -0.1],
      },
      {
        id: "back-stop-bolt",
        title: "Add the rear bolt",
        main: "rear-mount-thread#1",
        body: `Screw the rear bolt (M5×16) into the rear thread.`,
        instances: ["shcs-m5x16#1"],
        stage: [0, 0, -0.1],
      },
      {
        id: "belt-route",
        title: "Route the belt over the pulley and bearings",
        main: "gt2-20t-sh8#1",
        slide: 1.8, // the belt moves at half speed
        // Straight at the belt side, the whole device in view, to follow the routing.
        view: [-1, 0, 0],
        frame: "all",
        body: `Pass the belt over the bearings and under the pulley.`,
        instances: [],
        // Lifted to the bearings and fed through, pulling the sag under the rail straight.
        belt: { mode: "thread" },
      },
      {
        id: "back-stop-on-rail",
        title: "Thread the belt and push the rear thread onto the rail",
        main: "mgn15h-400mm-rail-part-1#1",
        view: [-0.58, 0.58, 0.58],
        slide: 1.8, // the belt moves at half speed
        body: `Thread the rear end of the belt through the rear thread, the same way as at the front, and push the rear thread onto the rail.`,
        instances: [],
        place: ["rear-mount-thread#1", "nut-m5#8", "shcs-m5x16#1"],
        // Into the rear clamp as it comes onto the rail; the spare sags until tensioned.
        belt: { mode: "rear", feed: [0, 1], slack: [0.0005, 0.0005] },
      },
      {
        id: "rear-nut-clamp",
        title: "Add the rear nut clamp",
        main: "rear-mount-thread#1",
        body: `Add the rear nut clamp.`,
        instances: ["tightener#2"],
      },
      {
        id: "back-belt-tensioner",
        title: "Add the back belt tensioner",
        main: "rear-mount-thread#1",
        body: `Add the back belt tensioner.`,
        instances: ["nut#2"],
      },
      {
        id: "back-stop-cover",
        title: "Fit the tensioner cover",
        main: "rear-mount-thread#1",
        body: `Finish with the tensioner cover.`,
        instances: ["cover#1"],
      },
      {
        id: "belt-tension",
        title: "Tension the belt",
        main: "rear-mount-thread#1",
        slide: 1.8, // the belt moves at half speed
        view: [-0.58, 0.58, -0.58],
        body: `Screw in the rear bolt (M5×16) to tighten the belt.`,
        instances: [],
        belt: { mode: "rear", feed: [1, 1], slack: [0.0005, 0] },
      },
      {
        id: "front-cover",
        title: "Fit the front cover",
        main: "main-mount-motor-side#1",
        view: [-0.58, 0.58, 0.58],
        body: `Fit the belt mechanics front cover over the dowel pins and bearings.`,
        instances: ["main-mount-top#1"],
      },
      {
        id: "front-cover-bolts",
        title: "Bolt the front cover",
        main: "main-mount-top#1",
        body: `Secure the front cover: two M5×30 and one M5×20 into the belt mechanics rear's M5 nuts, and two M3×12 into the rail carriage.`,
        instances: [
          "shcs-m5x30#5",
          "shcs-m5x30#6",
          "shcs-m5x20#1",
          "shcs-m3x12-mirrored#1",
          "shcs-m3x12-mirrored#2",
        ],
      },
    ],
  },
  {
    title: "Electronics and covers",
    steps: [
      {
        id: "motor-cables",
        title: "Plug the cables into the motor",
        main: "motor#1",
        view: [0.58, -0.58, 0.58],
        body: `Insert the connector cable and the RS485 cable into the motor.`,
        instances: ["degsonplug#1"],
      },
      {
        id: "spacer-nut",
        title: "Fit the M3 nut into the spacer",
        main: "motor-cover#1",
        // Back above the PCB end for the rest of the build.
        view: [0.6, 0.5, 0.6],
        body: `Press an M3 nut into the spacer.`,
        instances: ["motor-cover#1", "nut-m3#3"],
        stage: [0.1, 0, 0],
      },
      {
        id: "spacer",
        title: "Fit the spacer",
        main: "57aim-clamp#1",
        body: `Place the spacer on the main motor clamp.`,
        instances: [],
        place: ["motor-cover#1", "nut-m3#3"],
      },
      {
        id: "pcb-holder-nuts",
        title: "Fit the M3 nuts into the PCB holder",
        main: "pcbholder#1",
        view: [-0.58, 0.58, 0.58],
        body: `Press the two M3 nuts into the PCB holder. They're a tight fit on purpose: screwing a bolt into the nut and pressing on that helps. If one is loose, glue it.`,
        instances: ["pcbholder#1", "nut-m3#4", "nut-m3#5"],
        stage: [0.1, 0, 0],
        // Pressed in from the belt side.
        from: { "nut-m3#4": [-0.05, 0, 0], "nut-m3#5": [-0.05, 0, 0] },
      },
      {
        id: "pcb-holder",
        title: "Thread the cables through the PCB holder",
        main: "motor-cover#1",
        view: [0.6, 0.5, 0.6],
        body: `Thread the motor cables through the PCB holder and fit it onto the spacer.`,
        instances: [],
        place: ["pcbholder#1", "nut-m3#4", "nut-m3#5"],
      },
      {
        id: "pcb-holder-bolts",
        title: "Bolt the PCB holder and spacer",
        main: "pcbholder#1",
        body: `Secure the PCB holder and spacer with two M3×40 into the main motor clamp's M3 nuts.`,
        instances: ["shcs-m3x40#1", "shcs-m3x40#2"],
      },
      {
        id: "pcb",
        title: "Mount the PCB",
        main: "pcbholder#1",
        body: `Mount the main PCB on the PCB holder and secure it with two M3×8.`,
        instances: ["ossm-pcb", "shcs-m3x8#1", "shcs-m3x8#2"],
        // Behind the PCB, so they follow it in.
        from: { "shcs-m3x8#1": [0.1, 0, 0], "shcs-m3x8#2": [0.1, 0, 0] },
      },
      {
        id: "cover",
        title: "Close the cover",
        main: "pcbholder#1",
        body: `Press the LED cover into the top cover (glue it if it doesn't stay), then fix the cover with an M3×12 into the spacer's nut.`,
        instances: ["topcover#1", "ledcover#1", "shcs-m3x12#5"],
        // The bolt follows the cover in.
        from: { "shcs-m3x12#5": [0.3, 0, 0] },
      },
      {
        id: "finished",
        title: "Finished",
        main: "57aim-clamp#1",
        frame: "all",
        view: [-0.58, 0.58, 0.58],
        body: `That's it: your OSSM is assembled.`,
        instances: [],
      },
    ],
  },
];

export const GUIDE_STEPS: GuideStep[] = GUIDE.flatMap((s) => s.steps);
