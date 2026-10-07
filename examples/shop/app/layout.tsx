import type { ReactNode } from 'react';
import './globals.css';

export const metadata = { title: 'Kaapi Co. shop — Pratibimba example' };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        <nav>
          <b><a href="/" style={{ color: 'inherit', textDecoration: 'none' }}>Kaapi Co.</a></b>
          <a href="/p/monday-espresso">Espresso</a>
          <a href="/p/estate-filter">Filter</a>
        </nav>
        <main>{children}</main>
        <footer>An example store for Pratibimba. Nothing here is for sale.</footer>
      </body>
    </html>
  );
}
