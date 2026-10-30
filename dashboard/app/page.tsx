'use client';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import { ChevronRight, FilePen, Globe, LayoutGrid } from 'lucide-react';
import { useEffect } from 'react';
import { Shell } from '@/components/shell';
import { Card, Empty, Loading } from '@/components/ui';
import { api, type SiteRow } from '@/lib/api';

export default function Index() {
  const router = useRouter();
  const sites = useQuery({ queryKey: ['sites'], queryFn: () => api.get<SiteRow[]>('/sites') });
  useEffect(() => {
    if (sites.data?.length === 1) router.replace(`/site/?id=${sites.data[0].id}`);
  }, [sites.data, router]);
  return (
    <Shell site={null} title="Sites">
      {sites.isLoading && <Loading />}
      {sites.data?.length === 0 && <Empty icon={Globe}>No sites yet. Run <code className="font-mono">pratibimba apply site.json</code>.</Empty>}
      {!!sites.data?.length && (
        <Card className="divide-y divide-line">
          {sites.data.map((s) => (
            <Link key={s.id} href={`/site/?id=${s.id}`} className="flex items-center gap-3 px-4 py-3.5 transition-colors first:rounded-t-lg last:rounded-b-lg hover:bg-surface-2/60">
              <span className="grid size-8 place-items-center rounded-full border border-line bg-paper"><Globe className="size-4 text-ink-2" aria-hidden /></span>
              <div className="min-w-0 flex-1">
                <div className="font-medium">{s.id}</div>
                <div className="truncate text-[13px] text-muted">{s.origin}</div>
              </div>
              <span className="inline-flex items-center gap-1 text-[13px] text-ink-2 tabular" title="slots"><LayoutGrid className="size-3.5 text-muted" aria-label="slots" />{s.slots}</span>
              {s.pending > 0 && <span className="inline-flex items-center gap-1 text-[13px] text-warn-ink tabular" title="drafts waiting"><FilePen className="size-3.5" aria-label="drafts waiting" />{s.pending}</span>}
              <ChevronRight className="size-4 text-muted" aria-hidden />
            </Link>
          ))}
        </Card>
      )}
    </Shell>
  );
}
