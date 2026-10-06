'use client';
import * as S from '@radix-ui/react-slider';

/** The planned traffic share: 1% to 50%. The cap is also enforced by Core and the schema. */
export function ShareSlider({ value, onChange, max = 50, label }: { value: number; onChange: (v: number) => void; max?: number; label: string }) {
  return (
    <S.Root className="relative flex h-5 w-full touch-none items-center select-none" min={1} max={max} step={1} value={[value]} onValueChange={([v]) => onChange(v)} aria-label={label}>
      <S.Track className="relative h-1 grow rounded-full bg-surface-2">
        <S.Range className="absolute h-full rounded-full bg-accent" />
      </S.Track>
      <S.Thumb className="block size-4 rounded-full border-2 border-accent bg-surface shadow focus-visible:outline-2 focus-visible:outline-accent" />
    </S.Root>
  );
}
