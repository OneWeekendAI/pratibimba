const CART_KEY = 'pickly-cart';

const listeners = new Set();
const toastListeners = new Set();

function readStorage() {
  try {
    const raw = localStorage.getItem(CART_KEY);
    if (!raw) return { items: [] };
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed?.items)) return parsed;
    return { items: [] };
  } catch {
    return { items: [] };
  }
}

function writeStorage(cart) {
  try {
    localStorage.setItem(CART_KEY, JSON.stringify(cart));
  } catch {
    /* ignore storage quota / private browsing errors */
  }
  notify();
}

function notify() {
  const current = readStorage();
  listeners.forEach((cb) => {
    try {
      cb(current);
    } catch (err) {
      console.error('Error in cart change listener:', err);
    }
  });
}

// Listen to changes from other tabs
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === CART_KEY || e.key === null) {
      notify();
    }
  });
}

export function showToast(message) {
  toastListeners.forEach((cb) => {
    try {
      cb(message);
    } catch (err) {
      console.error('Error in toast listener:', err);
    }
  });
}

export function onToast(cb) {
  toastListeners.add(cb);
  return () => toastListeners.delete(cb);
}

export function getCart() {
  return readStorage();
}

const MAX_QTY_PER_JAR = 6;

export function addItem(id, qty = 1) {
  const cart = readStorage();
  const existing = cart.items.find((it) => it.id === id);
  if (existing) {
    existing.qty = Math.min(MAX_QTY_PER_JAR, existing.qty + qty);
  } else {
    cart.items.push({ id, qty: Math.min(MAX_QTY_PER_JAR, qty) });
  }
  writeStorage(cart);
  showToast('Added to cart');

  return cart;
}

export function removeItem(id) {
  const cart = readStorage();
  cart.items = cart.items.filter((it) => it.id !== id);
  writeStorage(cart);
  return cart;
}

export function setQty(id, qty) {
  const cart = readStorage();
  if (qty <= 0) {
    cart.items = cart.items.filter((it) => it.id !== id);
  } else {
    const cappedQty = Math.min(MAX_QTY_PER_JAR, qty);
    const existing = cart.items.find((it) => it.id === id);
    if (existing) {
      existing.qty = cappedQty;
    } else {
      cart.items.push({ id, qty: cappedQty });
    }
  }
  writeStorage(cart);
  return cart;
}

export function clearCart() {
  const cart = { items: [] };
  writeStorage(cart);
  return cart;
}

export function onChange(cb) {
  listeners.add(cb);
  // Initial call with current state
  try {
    cb(readStorage());
  } catch (err) {
    console.error('Error in cart initial callback:', err);
  }
  return () => {
    listeners.delete(cb);
  };
}
