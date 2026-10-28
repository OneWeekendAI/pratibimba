import { useEffect, useState } from 'react';
import Header from './components/Header';
import Footer from './components/Footer';
import Home from './components/Home';
import Shop from './components/Shop';
import ProductPage from './components/ProductPage';
import CartPage from './components/CartPage';
import CheckoutPage from './components/CheckoutPage';
import ThanksPage from './components/ThanksPage';
import AboutPage from './components/AboutPage';
import Toast from './components/Toast';
import { getCart, onChange } from './lib/cart';

function useHashRoute() {
  const [hash, setHash] = useState(() => window.location.hash || '#/');
  useEffect(() => {
    const onHashChange = () => {
      setHash(window.location.hash || '#/');
      window.scrollTo(0, 0);
    };
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);
  return hash;
}

export default function App() {
  const route = useHashRoute();
  const [cart, setCart] = useState(getCart);

  useEffect(() => {
    return onChange((updatedCart) => {
      setCart(updatedCart);
    });
  }, []);

  const cartCount = (cart.items || []).reduce((acc, it) => acc + (it.qty || 0), 0);

  let pageContent;

  if (route.startsWith('#/product/')) {
    const rawId = route.replace('#/product/', '').split('?')[0];
    pageContent = <ProductPage productId={rawId} />;
  } else if (route.startsWith('#/shop')) {
    pageContent = <Shop route={route} cartCount={cartCount} />;
  } else if (route.startsWith('#/cart')) {
    pageContent = <CartPage cart={cart} />;
  } else if (route.startsWith('#/checkout')) {
    pageContent = <CheckoutPage cart={cart} />;
  } else if (route.startsWith('#/thanks')) {
    pageContent = <ThanksPage />;
  } else if (route.startsWith('#/about')) {
    pageContent = <AboutPage />;
  } else {
    pageContent = <Home />;
  }

  return (
    <>
      <Header />
      <main id="main-content">{pageContent}</main>
      <Footer />
      <Toast />
    </>
  );
}
