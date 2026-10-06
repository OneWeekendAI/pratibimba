'use client';
import { useSearchParams } from 'next/navigation';

/** Routes carry their ids in the query string: a static export cannot prerender unknown paths. */
export function useParam(name: string): string | null {
  return useSearchParams().get(name);
}
