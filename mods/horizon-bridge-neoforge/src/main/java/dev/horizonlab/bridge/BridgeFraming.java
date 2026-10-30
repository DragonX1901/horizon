package dev.horizonlab.bridge;

import io.netty.channel.Channel;
import io.netty.util.AttributeKey;

/**
 * Framing for the bridge's connection. Mods register their own codecs for item components and
 * entity data, so the bridge cannot even skip a value it does not know. On the bridge's connection
 * only, each such value is written with its length in front ({@code DataComponentPatchCodecMixin},
 * {@code DataValueMixin}). The bridge then translates the values it knows and skips the rest.
 */
public final class BridgeFraming {
    /** Set on the channel of a connection that offered {@code horizon_bridge:hello}. */
    public static final AttributeKey<Boolean> BRIDGE = AttributeKey.valueOf("horizon_bridge:framing");
    /** Whether the packet being encoded on this thread goes to the bridge. */
    private static final ThreadLocal<Boolean> ENCODING_FOR_BRIDGE = ThreadLocal.withInitial(() -> false);

    private BridgeFraming() {}

    public static void encoding(Channel channel) {
        ENCODING_FOR_BRIDGE.set(Boolean.TRUE.equals(channel.attr(BRIDGE).get()));
    }

    public static boolean active() {
        return ENCODING_FOR_BRIDGE.get();
    }
}
