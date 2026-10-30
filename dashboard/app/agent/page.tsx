'use client';
// Build: connect a coding agent over MCP, and watch what it calls.
import { Suspense } from 'react';
import { ConnectAgentCard, McpTailCard } from '@/components/agent';
import { Shell } from '@/components/shell';
import { Empty } from '@/components/ui';
import { useParam } from '@/lib/page';

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title="Agent">
      {site ? (
        <div className="space-y-10">
          <ConnectAgentCard site={site} />
          <McpTailCard site={site} />
        </div>
      ) : (
        <Empty>No site given.</Empty>
      )}
    </Shell>
  );
}

export default function AgentPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
