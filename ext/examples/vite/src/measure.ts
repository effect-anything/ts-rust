// The canvas is this many millimetres across. Clip space is that length mapped to 1.
export const FRAME_MM = 160;

export class Length {
  millimeters: number;

  constructor(millimeters: number) {
    this.millimeters = millimeters;
  }
}

export function clipOf(span: Length, frame: Length): number {
  return span.millimeters / frame.millimeters;
}
