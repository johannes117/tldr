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

const rootRoute = createRootRoute({ component: Root });

const indexRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: Index });
const prRoute = createRoute({ getParentRoute: () => rootRoute, path: "/pr/$number", component: PrOverview });
const prFilesRoute = createRoute({ getParentRoute: () => rootRoute, path: "/pr/$number/files", component: PrFiles });
const prReviewRoute = createRoute({ getParentRoute: () => rootRoute, path: "/pr/$number/review", component: PrReview });

const routeTree = rootRoute.addChildren([indexRoute, prRoute, prFilesRoute, prReviewRoute]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register { router: typeof router }
}
