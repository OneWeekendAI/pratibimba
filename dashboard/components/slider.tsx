'use client';
import * as S from '@radix-ui/react-slider';

/** The planned traffic share: 1% to 50%. The cap is also enforced by Core and the schema. */
export function ShareSlider({ value, onChange, max = 50, label }: { value: number; onChange: (v: number) => void; max?: number; label: string }) {
  return (
    <S.Root className="relative flex h-5 w-full touch-none items-center select-none" min={1} max={max} step={1} value={[value]} onValueChange={([v]) => onChange(v)} aria-label={label}>
      <S.Track className="relative h-1 grow rounded-full bg-surface-3">
        <S.Range className="absolute h-full rounded-full bg-primary" />
      </S.Track>
      <S.Thumb className="block size-4 rounded-full border border-line-strong bg-surface shadow-[0_1px_3px_rgb(0_0_0/0.25)] transition-transform duration-150 hover:scale-110 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent max-sm:size-5" />
    </S.Root>
  );
}
