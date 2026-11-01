package dev.horizonlab.bridge;

import com.google.gson.*;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.jar.JarFile;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.geom.builders.LayerDefinition;

/**
 * Bakes the entity model layers mods define in Java (static methods returning a
 * {@code LayerDefinition}, as {@code createBodyLayer} does) into the part trees Horizon draws mobs
 * with: the same JSON as VanillaEntityModelExporter. Runs on NeoForge's Mojang-named 1.21.1 client
 * classes. Output: {@code {"<class>#<method>": part}}.
 *
 * <p>Usage: {@code ModModelExporter OUT.json MOD.jar...}
 */
public final class ModModelExporter {
    private static Field cubes;
    private static Field children;
    private static Field polygons;
    private static Field vertices;
    private static Field normal;
    private static Field pos;
    private static Field u;
    private static Field v;

    @SuppressWarnings("unchecked")
    private static JsonObject part(ModelPart part) throws Exception {
        var o = new JsonObject();
        var pose = new JsonArray();
        for (float f : new float[] {part.x, part.y, part.z, part.xRot, part.yRot, part.zRot, part.xScale, part.yScale, part.zScale})
            pose.add(f);
        o.add("pose", pose);
        var polys = new JsonArray();
        var normals = new JsonArray();
        for (Object cube : (List<Object>) cubes.get(part)) {
            for (Object polygon : (Object[]) polygons.get(cube)) {
                var p = new JsonArray();
                for (Object vertex : (Object[]) vertices.get(polygon)) {
                    var at = (org.joml.Vector3f) pos.get(vertex);
                    var vx = new JsonArray();
                    for (float f : new float[] {at.x(), at.y(), at.z(), u.getFloat(vertex), v.getFloat(vertex)})
                        vx.add(Float.isFinite(f) ? f : 0.0F);
                    p.add(vx);
                }
                polys.add(p);
                var n = (org.joml.Vector3f) normal.get(polygon);
                var nj = new JsonArray();
                nj.add(n.x());
                nj.add(n.y());
                nj.add(n.z());
                normals.add(nj);
            }
        }
        o.add("polygons", polys);
        o.add("normals", normals);
        var kids = new JsonObject();
        for (var entry : ((Map<String, ModelPart>) children.get(part)).entrySet())
            kids.add(entry.getKey(), part(entry.getValue()));
        o.add("children", kids);
        return o;
    }

    /** A field by name on `c` or a superclass, readable; null when none. */
    private static Field anyField(Class<?> c, String name) {
        for (; c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // Up a class.
            }
        }
        return null;
    }

    private static Object get(Object o, String name) throws Exception {
        Field f = anyField(o.getClass(), name);
        return f == null ? null : f.get(o);
    }

    private static float num(Object o, String name, float fallback) throws Exception {
        Object v = get(o, name);
        return v instanceof Number n ? n.floatValue() : fallback;
    }

    /** Whether `o` is a part of an older, constructor-built model (LLibrary's renderers). */
    private static boolean legacyPart(Object o) {
        return o != null && anyField(o.getClass(), "cubeList") != null && anyField(o.getClass(), "childModels") != null
                && anyField(o.getClass(), "rotationPointX") != null;
    }

    /** One constructor-built part, in the format of {@link #part}. */
    @SuppressWarnings("unchecked")
    private static JsonObject legacy(Object part, Map<Object, String> names) throws Exception {
        var o = new JsonObject();
        var pose = new JsonArray();
        for (float f : new float[] {num(part, "rotationPointX", 0), num(part, "rotationPointY", 0),
                num(part, "rotationPointZ", 0), num(part, "rotateAngleX", 0), num(part, "rotateAngleY", 0),
                num(part, "rotateAngleZ", 0), num(part, "scaleX", 1), num(part, "scaleY", 1), num(part, "scaleZ", 1)})
            pose.add(Float.isFinite(f) ? f : 0.0F);
        o.add("pose", pose);
        var polys = new JsonArray();
        var normals = new JsonArray();
        for (Object box : (Iterable<Object>) get(part, "cubeList")) {
            Object quads = get(box, "quads");
            if (quads == null) quads = get(box, "polygons");
            if (!(quads instanceof Object[] array)) continue;
            for (Object quad : array) {
                Object vertices = get(quad, "vertexPositions");
                if (!(vertices instanceof Object[] vs)) continue;
                var p = new JsonArray();
                for (Object vertex : vs) {
                    var at = (org.joml.Vector3f) get(vertex, "position");
                    var vx = new JsonArray();
                    for (float f : new float[] {at.x(), at.y(), at.z(), num(vertex, "textureU", 0), num(vertex, "textureV", 0)})
                        vx.add(Float.isFinite(f) ? f : 0.0F);
                    p.add(vx);
                }
                polys.add(p);
                var n = get(quad, "normal") instanceof org.joml.Vector3f v ? v : new org.joml.Vector3f(0, 1, 0);
                var nj = new JsonArray();
                nj.add(n.x());
                nj.add(n.y());
                nj.add(n.z());
                normals.add(nj);
            }
        }
        o.add("polygons", polys);
        o.add("normals", normals);
        var kids = new JsonObject();
        int i = 0;
        for (Object child : (Iterable<Object>) get(part, "childModels")) {
            kids.add(names.getOrDefault(child, "part" + i++), legacy(child, names));
        }
        o.add("children", kids);
        return o;
    }

    /**
     * A model built in its constructor (no layer definition): made with its no-argument
     * constructor, its parts found among its fields, the parts no other part holds as its roots.
     */
    @SuppressWarnings("unchecked")
    private static JsonObject legacyModel(Class<?> c) throws Exception {
        var made = c.getConstructor().newInstance();
        Map<Object, String> names = new java.util.IdentityHashMap<>();
        for (Class<?> k = c; k != null && k != Object.class; k = k.getSuperclass()) {
            for (Field f : k.getDeclaredFields()) {
                if (Modifier.isStatic(f.getModifiers())) continue;
                f.setAccessible(true);
                Object v = f.get(made);
                if (legacyPart(v)) names.putIfAbsent(v, f.getName());
            }
        }
        if (names.isEmpty()) return null;
        java.util.Set<Object> held = java.util.Collections.newSetFromMap(new java.util.IdentityHashMap<>());
        for (Object p : names.keySet()) {
            for (Object child : (Iterable<Object>) get(p, "childModels")) held.add(child);
        }
        var root = new JsonObject();
        var pose = new JsonArray();
        for (float f : new float[] {0, 0, 0, 0, 0, 0, 1, 1, 1}) pose.add(f);
        root.add("pose", pose);
        root.add("polygons", new JsonArray());
        root.add("normals", new JsonArray());
        var kids = new JsonObject();
        for (var e : names.entrySet()) {
            if (!held.contains(e.getKey())) kids.add(e.getValue(), legacy(e.getKey(), names));
        }
        root.add("children", kids);
        return root;
    }

    private static Field field(Class<?> c, String name) throws Exception {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    public static void main(String[] args) throws Exception {
        // No Bootstrap: layer definitions are plain mesh data, and NeoForge's
        // patched registries refuse to start outside its loader.
        cubes = field(ModelPart.class, "cubes");
        children = field(ModelPart.class, "children");
        Class<?> cube = Class.forName("net.minecraft.client.model.geom.ModelPart$Cube");
        Class<?> polygon = Class.forName("net.minecraft.client.model.geom.ModelPart$Polygon");
        Class<?> vertex = Class.forName("net.minecraft.client.model.geom.ModelPart$Vertex");
        polygons = field(cube, "polygons");
        vertices = field(polygon, "vertices");
        normal = field(polygon, "normal");
        pos = field(vertex, "pos");
        u = field(vertex, "u");
        v = field(vertex, "v");

        var jars = new ArrayList<Path>();
        for (int i = 1; i < args.length; i++) jars.add(Path.of(args[i]));
        var urls = new URL[jars.size()];
        for (int i = 0; i < urls.length; i++) urls[i] = jars.get(i).toUri().toURL();
        var loader = new URLClassLoader(urls, ModModelExporter.class.getClassLoader());
        var out = new JsonObject();
        var supers = new JsonObject();
        var renderers = new JsonObject();
        int failed = 0;
        for (Path jar : jars) {
            try (var file = new JarFile(jar.toFile())) {
                for (var entry : Collections.list(file.entries())) {
                    String name = entry.getName();
                    if (!name.endsWith(".class") || name.startsWith("META-INF") || name.contains("module-info")) continue;
                    String className = name.substring(0, name.length() - 6).replace('/', '.');
                    Class<?> c;
                    Method[] methods;
                    try {
                        c = Class.forName(className, false, loader);
                        methods = c.getDeclaredMethods();
                    } catch (Throwable error) {
                        continue;
                    }
                    // A model whose layer its parent class defines (Aether's CockatriceModel and
                    // BipedBirdModel.createBodyLayer): matched through this map.
                    try {
                        if (net.minecraft.client.model.Model.class.isAssignableFrom(c) && c.getSuperclass() != null) {
                            supers.addProperty(c.getName(), c.getSuperclass().getName());
                        }
                    } catch (Throwable error) {
                        // Left out.
                    }
                    // A renderer drawing a vanilla model (Aether's FlyingCowRenderer and CowModel):
                    // the model class its superclass's type arguments name.
                    try {
                        if (net.minecraft.client.renderer.entity.EntityRenderer.class.isAssignableFrom(c)
                                && c.getGenericSuperclass() instanceof java.lang.reflect.ParameterizedType t) {
                            for (var arg : t.getActualTypeArguments()) {
                                Class<?> raw = arg instanceof Class<?> k ? k
                                        : arg instanceof java.lang.reflect.ParameterizedType p
                                                && p.getRawType() instanceof Class<?> k2 ? k2 : null;
                                if (raw != null && net.minecraft.client.model.Model.class.isAssignableFrom(raw)) {
                                    renderers.addProperty(c.getName(), raw.getName());
                                }
                            }
                        }
                    } catch (Throwable error) {
                        // Left out.
                    }
                    // Constructor-built models (LLibrary's, Mowzie's mobs).
                    try {
                        if (net.minecraft.client.model.Model.class.isAssignableFrom(c)
                                && !Modifier.isAbstract(c.getModifiers())
                                && java.util.Arrays.stream(methods).noneMatch(m -> m.getReturnType() == LayerDefinition.class)) {
                            c.getConstructor();
                            var legacy = legacyModel(c);
                            if (legacy != null) out.add(c.getName() + "#<init>", legacy);
                        }
                    } catch (Throwable error) {
                        // Not made without its arguments, or not built that way.
                    }
                    for (Method m : methods) {
                        if (!Modifier.isStatic(m.getModifiers()) || m.getReturnType() != LayerDefinition.class) continue;
                        Object[] arguments = defaults(m.getParameterTypes());
                        if (arguments == null) continue;
                        try {
                            m.setAccessible(true);
                            var layer = (LayerDefinition) m.invoke(null, arguments);
                            out.add(c.getName() + "#" + m.getName(), part(layer.bakeRoot()));
                        } catch (Throwable error) {
                            if (failed++ < 3) {
                                Throwable cause = error.getCause() != null ? error.getCause() : error;
                                System.err.println("failed " + c.getName() + "#" + m.getName() + ": " + cause);
                            }
                        }
                    }
                }
            }
        }
        out.add("__supers__", supers);
        out.add("__renderers__", renderers);

        Files.writeString(Path.of(args[0]), new Gson().toJson(out), StandardCharsets.UTF_8);
        System.out.println(out.size() + " layers, " + failed + " failed");
    }

    /**
     * The arguments a layer method takes as its default look: no cube deformation (the
     * {@code createBodyLayer(CubeDeformation)} pattern), zeros and false; null when it takes
     * something else.
     */
    private static Object[] defaults(Class<?>[] types) {
        Object[] out = new Object[types.length];
        for (int i = 0; i < types.length; i++) {
            Class<?> t = types[i];
            if (t == net.minecraft.client.model.geom.builders.CubeDeformation.class) {
                out[i] = net.minecraft.client.model.geom.builders.CubeDeformation.NONE;
            } else if (t == float.class) {
                out[i] = 0f;
            } else if (t == double.class) {
                out[i] = 0d;
            } else if (t == int.class) {
                out[i] = 0;
            } else if (t == boolean.class) {
                out[i] = false;
            } else {
                return null;
            }
        }
        return out;
    }
}
