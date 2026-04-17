import {
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { Root } from "./routes/__root";
import { Index } from "./routes/index";
import { PrOverview } from "./routes/pr.$number";
import { PrFiles } from "./routes/pr.$number.files";
import { PrReview } from "./routes/pr.$number.review";
import { PrWalkthrough } from "./routes/pr.$number.walkthrough";
import { PrCoverage } from "./routes/pr.$number.coverage";
import { PrGraph } from "./routes/pr.$number.graph";
import { PrBlast } from "./routes/pr.$number.blast.$symbol";

const rootRoute = createRootRoute({ component: Root });

const indexRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: Index });
const prRoute = createRoute({ getParentRoute: () => rootRoute, path: "/pr/$number", component: PrOverview });
const prFilesRoute = createRoute({ getParentRoute: () => rootRoute, path: "/pr/$number/files", component: PrFiles });
const prReviewRoute = createRoute({ getParentRoute: () => rootRoute, path: "/pr/$number/review", component: PrReview });
const prWalkthroughRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/pr/$number/walkthrough",
  component: PrWalkthrough,
  validateSearch: (s: Record<string, unknown>) => ({ view: typeof s.view === "string" ? s.view : undefined }),
});

const prCoverageRoute = createRoute({ getParentRoute: () => rootRoute, path: "/pr/$number/coverage", component: PrCoverage });

const prGraphRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/pr/$number/graph",
  component: PrGraph,
  validateSearch: (s: Record<string, unknown>) => ({
    hide: typeof s.hide === "string" ? s.hide : undefined,
    hop: typeof s.hop === "string" ? s.hop : undefined,
  }),
});
const prBlastRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/pr/$number/blast/$symbol",
  component: PrBlast,
});

const routeTree = rootRoute.addChildren([indexRoute, prRoute, prFilesRoute, prReviewRoute, prWalkthroughRoute, prCoverageRoute, prGraphRoute, prBlastRoute]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register { router: typeof router }
}
