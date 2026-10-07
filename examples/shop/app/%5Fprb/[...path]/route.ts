// Served at /_prb/* (a folder named "_prb" would be private in the App Router, hence %5F).
import { prb } from '@/lib/prb';

export const { GET, POST } = prb.route;
export const dynamic = 'force-dynamic';
