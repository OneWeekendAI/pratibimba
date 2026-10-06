import Link from 'next/link';

export default function NotFound() {
  return (
    <div className="grid min-h-screen place-items-center">
      <div className="text-center">
        <p className="mb-2 font-serif text-2xl">Nothing here</p>
        <Link href="/" className="text-accent underline">Back to the dashboard</Link>
      </div>
    </div>
  );
}
