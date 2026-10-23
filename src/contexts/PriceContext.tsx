// Modified by Klimper (2026): see docs/UPSTREAM.md
import { useNetwork } from '@/hooks/useNetwork';
import { fetchPrices as fetchKlimperPrices } from '@/klimper/price';
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
  getBalanceInUsd: (assetId: string | null, balance: string) => string;
  getPriceInUsd: (assetId: string | null) => number;
  getCatAskPriceInXch: (assetId: string) => number | null;
  isLoading: boolean;
}

export const PriceContext = createContext<PriceContextType | undefined>(
  undefined,
);

export function PriceProvider({ children }: { children: ReactNode }) {
  const [xchUsdPrice, setXchUsdPrice] = useState<number>(0);
  const [catPrices, setCatPrices] = useState<Record<string, CatPriceData>>({});
  const { network, isTestnet } = useNetwork();
  const [isPriceLoading, setIsPriceLoading] = useState(false);
  const intervalRef = useRef<NodeJS.Timeout | null>(null);

  useEffect(() => {
    // Don't fetch prices until network is loaded
    if (network === null) {
      return;
    }

    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }

    // Klimper: one price source for every asset (src/klimper/price.ts, TASK-113): nodexch on
    // mainnet with CoinGecko and dexie as the fallback, zero dollars on testnet.
    const fetchPrices = async () => {
      setIsPriceLoading(true);
      try {
        const prices = await fetchKlimperPrices(isTestnet);
        setXchUsdPrice(prices.xchUsd);
        setCatPrices(prices.cats);
      } finally {
        setIsPriceLoading(false);
      }
    };

    fetchPrices();
    intervalRef.current = setInterval(fetchPrices, 60000);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [network, isTestnet]);

  const getPriceInUsd = useCallback(
    (assetId: string | null) => {
      if (!assetId) return xchUsdPrice;

      const priceData = catPrices[assetId.toLowerCase()];
      const xchPrice = priceData?.lastPrice;

      if (xchPrice === null || xchPrice === undefined) {
        return 0;
      }

      return xchPrice * xchUsdPrice;
    },
    [xchUsdPrice, catPrices],
  );

  const getBalanceInUsd = useCallback(
    (assetId: string | null, balance: string) => {
      // Validate balance input
      const balanceNum = Number(balance);
      if (isNaN(balanceNum)) {
        return '0.00';
      }

      if (!assetId) return (balanceNum * xchUsdPrice).toFixed(2);

      const priceData = catPrices[assetId.toLowerCase()];
      const xchPrice = priceData?.lastPrice;

      if (xchPrice === null || xchPrice === undefined) {
        return '0.00';
      }

      return (balanceNum * xchPrice * xchUsdPrice).toFixed(2);
    },
    [xchUsdPrice, catPrices],
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
        getBalanceInUsd,
        getPriceInUsd,
        getCatAskPriceInXch,
        isLoading: network === null || isPriceLoading,
      }}
    >
      {children}
    </PriceContext.Provider>
  );
}
