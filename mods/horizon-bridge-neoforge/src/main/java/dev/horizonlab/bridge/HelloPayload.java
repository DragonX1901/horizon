package dev.horizonlab.bridge;

import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;

/** {@code horizon_bridge:hello}: only its presence in the client's channel list matters. */
public final class HelloPayload implements CustomPacketPayload {
    public static final Type<HelloPayload> TYPE = new Type<>(HorizonBridge.id("hello"));
    public static final HelloPayload INSTANCE = new HelloPayload();
    public static final StreamCodec<ByteBuf, HelloPayload> STREAM_CODEC = StreamCodec.unit(INSTANCE);

    private HelloPayload() {}

    @Override
    public Type<HelloPayload> type() {
        return TYPE;
    }
}
