import ShelfJar from './ShelfJar';

/*
 * A run of teak planks. Each child draws its own plank segment, so the
 * grid's direct children stay one-per-pickle (keyed by id). Planks join
 * because columns have no gap; brackets show at each row's ends via CSS.
 */
export default function ShelfGrid({ pickles, id, isBack, pulse = 0, variant = 'full', headingLevel = 3 }) {
  return (
    <div className={`shelf-wall shelf-wall--${variant}`}>
      <div className="shelf-scroll">
        <div id={id} className="shelf-grid">
          {pickles.map((p, i) => (
            <ShelfJar
              key={p.id}
              pickle={p}
              index={i}
              back={isBack ? isBack(p) : false}
              pulse={pulse}
              headingLevel={headingLevel}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
