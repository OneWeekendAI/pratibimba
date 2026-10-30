import Link from 'next/link';

export default function NotFound() {
  return (
    <div className="grid min-h-screen place-items-center bg-paper px-4">
      <div className="text-center">
        <p className="font-mono text-[13px] text-muted">404</p>
        <p className="mt-2 mb-4 text-[22px] font-semibold tracking-[-0.03em]">Nothing here</p>
        <Link href="/" className="text-[13px] font-medium text-accent hover:underline">Back to the dashboard</Link>
      </div>
    </div>
  );
}
