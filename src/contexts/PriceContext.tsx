// Modified by Klimper (2026): see docs/UPSTREAM.md
import { useNetwork } from '@/hooks/useNetwork';
import { FiatCurrency, usePriceSettings } from '@/hooks/usePriceSettings';
import {
  fetchPrices as fetchKlimperPrices,
  resetPrices,
} from '@/klimper/price';
import {
  createContext,
  ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';

// Add an interface for the price data structure
interface CatPriceData {
  lastPrice: number | null;
  askPrice: number | null;
}

export interface PriceContextType {
  getBalanceInFiat: (assetId: string | null, balance: string) => string;
  getPriceInFiat: (assetId: string | null) => number;
  getCatAskPriceInXch: (assetId: string) => number | null;
  /** The currency that fiat values are in, or null when price lookups are off and fiat values are
   *  all zero. */
  fiatCurrency: FiatCurrency | null;
  isLoading: boolean;
}

export const PriceContext = createContext<PriceContextType | undefined>(
  undefined,
);

export function PriceProvider({ children }: { children: ReactNode }) {
  const { currency, pricesEnabled } = usePriceSettings();
  const [xchFiatPrice, setXchFiatPrice] = useState<number>(0);
  const [catPrices, setCatPrices] = useState<Record<string, CatPriceData>>({});
  const { network, isTestnet } = useNetwork();
  const [isPriceLoading, setIsPriceLoading] = useState(false);
  const intervalRef = useRef<NodeJS.Timeout | null>(null);

  useEffect(() => {
    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }

    // A price in another currency, or one from before lookups were turned off, is wrong now.
    setXchFiatPrice(0);

    if (!pricesEnabled) {
      setCatPrices({});
      resetPrices(); // Klimper: the sound and approvals read these too
      return;
    }

    // Don't fetch prices until network is loaded
    if (network === null) {
      return;
    }

    // A response that arrives after the currency changes is for the old one.
    let cancelled = false;

    // Klimper: one price source for every asset (src/klimper/price.ts, TASK-113): nodexch on
    // mainnet with CoinGecko and dexie as the fallback, zero on testnet.
    const fetchPrices = async () => {
      setIsPriceLoading(true);
      try {
        const prices = await fetchKlimperPrices(isTestnet, currency);
        if (cancelled) return;
        setXchFiatPrice(prices.xchFiat);
        setCatPrices(prices.cats);
      } finally {
        if (!cancelled) setIsPriceLoading(false);
      }
    };

    fetchPrices();
    intervalRef.current = setInterval(fetchPrices, 60000);

    return () => {
      cancelled = true;
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [network, isTestnet, currency, pricesEnabled]);

  const getPriceInFiat = useCallback(
    (assetId: string | null) => {
      if (!assetId) return xchFiatPrice;

      const priceData = catPrices[assetId.toLowerCase()];
      const xchPrice = priceData?.lastPrice;

      if (xchPrice === null || xchPrice === undefined) {
        return 0;
      }

      return xchPrice * xchFiatPrice;
    },
    [xchFiatPrice, catPrices],
  );

  const getBalanceInFiat = useCallback(
    (assetId: string | null, balance: string) => {
      // Validate balance input
      const balanceNum = Number(balance);
      if (isNaN(balanceNum)) {
        return '0.00';
      }

      if (!assetId) return (balanceNum * xchFiatPrice).toFixed(2);

      const priceData = catPrices[assetId.toLowerCase()];
      const xchPrice = priceData?.lastPrice;

      if (xchPrice === null || xchPrice === undefined) {
        return '0.00';
      }

      return (balanceNum * xchPrice * xchFiatPrice).toFixed(2);
    },
    [xchFiatPrice, catPrices],
  );

  const getCatAskPriceInXch = useCallback(
    (assetId: string) => {
      const priceData = catPrices[assetId.toLowerCase()];
      return priceData?.askPrice ?? null;
    },
    [catPrices],
  );

  return (
    <PriceContext.Provider
      value={{
        getBalanceInFiat,
        getPriceInFiat,
        getCatAskPriceInXch,
        fiatCurrency: pricesEnabled ? currency : null,
        isLoading: pricesEnabled && (network === null || isPriceLoading),
      }}
    >
      {children}
    </PriceContext.Provider>
  );
}
