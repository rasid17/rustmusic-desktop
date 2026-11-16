/**
 * Inertial disc rotation clock (`SpinClock`).
 *
 * Accelerates smoothly up to `targetSpeed` (deg/s) while `running` is true,
 * and coasts smoothly to a stop (decelerating by `decel` deg/s²) when `running`
 * becomes false — holding its final angle instead of snapping back to 0°.
 * Once settled (`velocity === 0 && !running`), the `requestAnimationFrame` loop
 * stops completely so idle CPU usage is 0%.
 */
export class SpinClock {
  private angle = 0;
  private velocity = 0;
  private running = false;
  private rafId = 0;
  private lastTs = 0;
  private onUpdate: (angle: number) => void;

  /** Target angular velocity in degrees per second (24 deg/s = 15s per revolution). */
  private readonly targetSpeed = 24;
  /** Spin-up acceleration in degrees per second². */
  private readonly accel = 52;
  /** Coast-to-stop deceleration in degrees per second². */
  private readonly decel = 34;

  constructor(onUpdate: (angle: number) => void) {
    this.onUpdate = onUpdate;
  }

  setRunning(running: boolean): void {
    if (this.running === running) return;
    this.running = running;
    this.queueFrame();
  }

  reset(): void {
    this.angle = 0;
    this.velocity = 0;
    this.onUpdate(0);
  }

  destroy(): void {
    this.running = false;
    if (this.rafId) {
      cancelAnimationFrame(this.rafId);
      this.rafId = 0;
    }
  }

  private get isSettled(): boolean {
    return !this.running && this.velocity <= 0.001;
  }

  private queueFrame(): void {
    if (this.rafId || this.isSettled) return;
    this.lastTs = performance.now();
    this.rafId = requestAnimationFrame(this.tick);
  }

  private tick = (now: number): void => {
    this.rafId = 0;
    // Clamp elapsed time so a hidden window or sleep doesn't whip the disc round.
    const dt = Math.min(Math.max(0, (now - this.lastTs) / 1000), 0.1);
    this.lastTs = now;

    if (this.running) {
      this.velocity = Math.min(this.targetSpeed, this.velocity + this.accel * dt);
    } else {
      this.velocity = Math.max(0, this.velocity - this.decel * dt);
    }

    if (this.velocity > 0) {
      this.angle = (this.angle + this.velocity * dt) % 360;
      this.onUpdate(this.angle);
    }

    if (!this.isSettled) {
      this.rafId = requestAnimationFrame(this.tick);
    }
  };
}
