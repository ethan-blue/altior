import React, { createContext, useContext, useEffect, useMemo } from "react";
import { en } from "./en";
import { zhCN } from "./zh-CN";
import type { LocaleSource, SupportedLocale, TranslationDictionary } from "./types";

export * from "./types";

export const dictionaries: Record<SupportedLocale, TranslationDictionary> = {
  en,
  "zh-CN": zhCN,
};

export function getSystemLocale(): SupportedLocale {
  if (typeof navigator !== "undefined" && typeof navigator.language === "string") {
    const lang = navigator.language.toLowerCase();
    if (lang.startsWith("zh")) {
      return "zh-CN";
    }
  }
  return "en";
}

export function resolveLocale(source: LocaleSource): SupportedLocale {
  if (source === "system") {
    return getSystemLocale();
  }
  return source === "zh-CN" ? "zh-CN" : "en";
}

export function getDictionary(locale: SupportedLocale): TranslationDictionary {
  return dictionaries[locale] ?? dictionaries.en;
}

export interface I18nContextValue {
  readonly t: TranslationDictionary;
  readonly locale: SupportedLocale;
  readonly localeSource: LocaleSource;
  readonly setLocaleSource: (source: LocaleSource) => void;
}

const I18nContext = createContext<I18nContextValue>({
  t: en,
  locale: "en",
  localeSource: "en",
  setLocaleSource: () => {},
});

export interface I18nProviderProps {
  readonly localeSource: LocaleSource;
  readonly onLocaleSourceChange?: (source: LocaleSource) => void;
  readonly children: React.ReactNode;
}

export function I18nProvider({
  localeSource,
  onLocaleSourceChange,
  children,
}: I18nProviderProps) {
  const resolvedLocale = useMemo(() => resolveLocale(localeSource), [localeSource]);
  const dictionary = useMemo(() => getDictionary(resolvedLocale), [resolvedLocale]);

  useEffect(() => {
    if (typeof document !== "undefined" && document.documentElement) {
      document.documentElement.lang = resolvedLocale;
    }
  }, [resolvedLocale]);

  const value = useMemo<I18nContextValue>(
    () => ({
      t: dictionary,
      locale: resolvedLocale,
      localeSource,
      setLocaleSource: (src) => onLocaleSourceChange?.(src),
    }),
    [dictionary, resolvedLocale, localeSource, onLocaleSourceChange],
  );

  return React.createElement(I18nContext.Provider, { value }, children);
}

export function useI18n(): I18nContextValue {
  return useContext(I18nContext);
}
