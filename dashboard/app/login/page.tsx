'use client';
import { useState, Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import { KeyRound } from 'lucide-react';
import { Mark } from '@/components/shell';
import { Button, Card, Field, Help, inputClass } from '@/components/ui';
import { api, ApiError, Unauthorized } from '@/lib/api';

function Login() {
  const next = useSearchParams().get('next');
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <div className="grid min-h-screen place-items-center bg-paper px-4">
      <div className="w-full max-w-[22rem]">
        <div className="mb-8 flex flex-col items-center gap-4 text-center">
          <Mark className="size-9 text-ink" />
          <div>
            <h1 className="text-[22px] font-semibold tracking-[-0.03em]">Sign in to Pratibimba</h1>
            <p className="mt-1 inline-flex items-center gap-0.5 text-[13px] text-muted">
              With the server&apos;s admin token
              <Help title="Admin token">
                <p>The token this server was started with, in <code className="font-mono text-xs">PRB_ADMIN_TOKEN</code>.</p>
              </Help>
            </p>
          </div>
        </div>
      <Card className="p-6">
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError(null);
            try {
              await api.login(token);
              location.assign(next && next.startsWith('/app/') && !next.startsWith('/app/login') ? next : '/app/');
            } catch (err) {
              setError(err instanceof Unauthorized ? 'That token was not accepted.' : err instanceof ApiError ? err.message : 'Could not reach Core.');
              setBusy(false);
            }
          }}
          className="space-y-5"
        >
          <Field label="Admin token">
            <input type="password" autoComplete="current-password" className={`${inputClass} w-full`} value={token} onChange={(e) => setToken(e.target.value)} autoFocus />
          </Field>
          {error && <p role="alert" className="text-[13px] text-pause-ink">{error}</p>}
          <Button type="submit" disabled={!token || busy} className="w-full" icon={KeyRound}>Sign in</Button>
        </form>
      </Card>
      </div>
    </div>
  );
}

export default function Page() {
  return (
    <Suspense>
      <Login />
    </Suspense>
  );
}
