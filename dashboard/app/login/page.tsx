'use client';
import { useState, Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import { Button, Card, Field, inputClass } from '@/components/ui';
import { api, ApiError, Unauthorized } from '@/lib/api';

function Login() {
  const next = useSearchParams().get('next');
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <div className="grid min-h-screen place-items-center bg-chrome px-4">
      <Card className="w-full max-w-sm p-6">
        <h1 className="mb-1 font-serif text-2xl">Pratibimba</h1>
        <p className="mb-5 text-[13px] text-ink-2">Paste the admin token this server was started with (<code className="font-mono text-xs">PRB_ADMIN_TOKEN</code>).</p>
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
          className="space-y-4"
        >
          <Field label="Admin token">
            <input type="password" autoComplete="current-password" className={`${inputClass} w-full w-full`} value={token} onChange={(e) => setToken(e.target.value)} autoFocus />
          </Field>
          {error && <p role="alert" className="text-[13px] text-pause-ink">{error}</p>}
          <Button type="submit" disabled={!token || busy} className="w-full">Sign in</Button>
        </form>
      </Card>
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
