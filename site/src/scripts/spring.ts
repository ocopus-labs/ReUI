// ReUI's springs, as Windows.UI.Composition runs them (design/tokens.json motion.spring):
// a damping ratio and a period in ms, the time one undamped oscillation would take.
export interface Preset {
  damping: number;
  period: number;
}

export const springs = {
  expand: { damping: 1.0, period: 60 },
  collapse: { damping: 0.92, period: 55 },
  peek: { damping: 0.9, period: 45 },
  settle: { damping: 0.95, period: 55 },
  reduced: { damping: 1.0, period: 40 },
} satisfies Record<string, Preset>;

// Composition's period is short and its springs read slower than the textbook ones on a
// 60 Hz page; this is the one tuning knob between the two, set by eye against the app.
const TIME = 0.16;

export class Spring {
  value: number;
  target: number;
  velocity = 0;
  preset: Preset = springs.expand;

  constructor(value: number) {
    this.value = value;
    this.target = value;
  }

  to(target: number, preset: Preset) {
    this.target = target;
    this.preset = preset;
  }

  /** Advances by `dt` seconds; true while still moving. */
  step(dt: number): boolean {
    const omega = (2 * Math.PI) / (this.preset.period / 1000) * TIME;
    const zeta = this.preset.damping;
    // Small fixed steps keep the stiff spring stable at any frame rate.
    for (let left = dt; left > 0; left -= 1 / 480) {
      const h = Math.min(left, 1 / 480);
      const accel = -omega * omega * (this.value - this.target) - 2 * zeta * omega * this.velocity;
      this.velocity += accel * h;
      this.value += this.velocity * h;
    }
    const settled = Math.abs(this.value - this.target) < 0.05 && Math.abs(this.velocity) < 0.5;
    if (settled) {
      this.value = this.target;
      this.velocity = 0;
    }
    return !settled;
  }
}
