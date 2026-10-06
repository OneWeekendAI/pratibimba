'use client';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
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
    <Shell site={null} title={<h1 className="font-serif text-3xl">Sites</h1>}>
      {sites.isLoading && <Loading />}
      {sites.data?.length === 0 && <Empty>No sites yet. Declare one with <code>pratibimba apply site.json</code>.</Empty>}
      <div className="grid gap-3 sm:grid-cols-2">
        {sites.data?.map((s) => (
          <Link key={s.id} href={`/site/?id=${s.id}`}>
            <Card className="p-4 hover:border-line-strong">
              <div className="font-mono text-sm">{s.id}</div>
              <div className="text-xs text-muted">{s.origin} · {s.slots} slots · {s.pending} drafts waiting</div>
            </Card>
          </Link>
        ))}
      </div>
    </Shell>
  );
}
