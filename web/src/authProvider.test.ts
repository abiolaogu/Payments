import { beforeEach, describe, expect, it, vi } from "vitest";
import { authProvider, getAccessToken } from "./authProvider";

describe("authProvider", () => {
  beforeEach(async () => {
    await authProvider.logout({});
  });

  it("fails closed when a verified token is absent", async () => {
    const result = await authProvider.login({ token: "" });

    expect(result.success).toBe(false);
    expect(getAccessToken()).toBeNull();
    await expect(authProvider.check({})).resolves.toMatchObject({ authenticated: false });
  });

  it("keeps the access token in memory and clears it on logout", async () => {
    await expect(authProvider.login({ token: "verified-token" })).resolves.toMatchObject({
      success: true,
    });
    expect(getAccessToken()).toBe("verified-token");
    expect(vi.mocked(window.localStorage.setItem)).not.toHaveBeenCalled();

    await authProvider.logout({});
    expect(getAccessToken()).toBeNull();
  });
});
