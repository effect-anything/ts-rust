

export interface Record {
  readonly id: string;
  readonly createdAt: string;
    readonly updatedAt: string;
  parse(value: string): string;
  parse(value: number): number;
}
