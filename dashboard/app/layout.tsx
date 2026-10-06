import type { ReactNode } from 'react';
import { Providers } from '@/components/providers';
// Self-hosted faces: a self-hosted dashboard should not call a font CDN.
import '@fontsource/ibm-plex-sans/400.css';
import '@fontsource/ibm-plex-sans/500.css';
import '@fontsource/ibm-plex-sans/600.css';
import '@fontsource-variable/fraunces';
import '@fontsource/jetbrains-mono/400.css';
import './globals.css';

export const metadata = { title: 'Pratibimba', description: 'Personalisation you can read' };

// The saved theme is applied before first paint, so the page never flashes the other one.
const themeScript = `try{var t=localStorage.getItem('prb-theme');if(t)document.documentElement.dataset.theme=t}catch(e){}`;

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
