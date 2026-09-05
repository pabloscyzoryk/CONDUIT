

import { useCallback, useState, type ReactNode } from "react";
import { Button, Icon, Modal } from "@/components/ui";
import { useT } from "@/i18n";

export interface PytaniePotwierdzenia {
  tytul: string;
  tresc: ReactNode;
  /** Dopisuje zdanie „tej operacji nie da się cofnąć" i czerwony przycisk. */
  nieodwracalne?: boolean;
  /** Etykieta przycisku wykonania; domyślnie „Wykonaj". */
  etykieta?: string;
  onTak: () => void;
}

/**
 * Zwraca `zapytaj(...)` oraz gotowe `okno` do wstawienia w drzewo.
 * Wywołujący nie trzyma żadnego stanu — jedno pytanie naraz wystarcza,
 * bo okno jest modalne.
 */
export function usePotwierdzenie(): { zapytaj: (p: PytaniePotwierdzenia) => void; okno: ReactNode } {
  const tt = useT();
  const [pytanie, setPytanie] = useState<PytaniePotwierdzenia | null>(null);

  const zapytaj = useCallback((p: PytaniePotwierdzenia) => setPytanie(p), []);
  const zamknij = useCallback(() => setPytanie(null), []);

  const okno = (
    <Modal
      open={pytanie !== null}
      onClose={zamknij}
      title={pytanie?.tytul ?? tt("potw.title")}
      width={480}
      footer={
        <>
          <Button variant="ghost" onClick={zamknij}>
            {tt("potw.no")}
          </Button>
          <Button
            variant={pytanie?.nieodwracalne ? "danger" : "primary"}
            icon={pytanie?.nieodwracalne ? "alert" : "check"}
            onClick={() => {
              pytanie?.onTak();
              zamknij();
            }}
          >
            {pytanie?.etykieta ?? tt("potw.yes")}
          </Button>
        </>
      }
    >
      <div className="potw">
        <p>{pytanie?.tresc}</p>
        {pytanie?.nieodwracalne && (
          <p className="potw__hard">
            <Icon name="alert" size={13} />
            {tt("potw.irreversible")}
          </p>
        )}
      </div>
    </Modal>
  );

  return { zapytaj, okno };
}
