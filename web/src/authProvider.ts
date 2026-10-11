import type { AuthBindings } from "@refinedev/core";

let accessToken: string | null = null;

export function getAccessToken(): string | null {
  return accessToken;
}

export const authProvider: AuthBindings = {
  login: async ({ token }) => {
    if (typeof token !== "string" || token.trim().length === 0) {
      return {
        success: false,
        error: {
          name: "AuthenticationError",
          message: "A verified StratoID/Authentik access token is required",
        },
      };
    }
    accessToken = token.trim();
    return { success: true, redirectTo: "/" };
  },
  logout: async () => {
    accessToken = null;
    return { success: true, redirectTo: "/login" };
  },
  check: async () => {
    if (accessToken) {
      return { authenticated: true };
    }
    return { authenticated: false, redirectTo: "/login" };
  },
  getPermissions: async () => null,
  getIdentity: async () => null,
  onError: async () => ({ error: undefined }),
};
