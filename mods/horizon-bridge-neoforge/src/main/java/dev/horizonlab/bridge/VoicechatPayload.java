package dev.horizonlab.bridge;

import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.neoforged.fml.ModList;

/**
 * {@code horizon_bridge:voicechat}: Simple Voice Chat's compatibility version on this server
 * ({@code Voicechat.COMPATIBILITY_VERSION}), which its clients must ask for their secret with and
 * which decides its packets' layout. Sent only when the mod is loaded. Read by neoforge.rs.
 */
public record VoicechatPayload() implements CustomPacketPayload {
    public static final Type<VoicechatPayload> TYPE = new Type<>(HorizonBridge.id("voicechat"));
    public static final StreamCodec<FriendlyByteBuf, VoicechatPayload> STREAM_CODEC =
            StreamCodec.of((buf, payload) -> buf.writeInt(compatibility()), buf -> {
                throw new UnsupportedOperationException("The voice chat version is only sent by the server");
            });

    public static boolean loaded() {
        return ModList.get().isLoaded("voicechat") && compatibility() >= 0;
    }

    static int compatibility() {
        try {
            return Class.forName("de.maxhenkel.voicechat.Voicechat").getField("COMPATIBILITY_VERSION").getInt(null);
        } catch (ReflectiveOperationException | LinkageError e) {
            return -1;
        }
    }

    @Override
    public Type<VoicechatPayload> type() {
        return TYPE;
    }
}
