/*
 * Created on 3/02/2005
 */
package net.pengo.hexdraw.layout;

import java.awt.Rectangle;
import java.util.Collections;
import java.util.LinkedList;
import java.util.List;

import net.pengo.bitSelection.BitCursor;

/**
 * @author Peter Halasz
 */
public class LayoutCursor {

	/* use pathList preferably, but if setPath() is used pathList will be lazily created */
	private Object[] path;
	private LinkedList pathList;
	
	protected BitCursor bitLocation;
	protected Rectangle blinkyLocation;

	private static LayoutCursor unactiveCursor;
	public static LayoutCursor unactiveCursor() {
		if (unactiveCursor == null) {
			unactiveCursor = new LayoutCursor() {
				public LayoutCursorDescender descender() {
					return LayoutCursorDescender.unactiveDescender();
				}
			};
			unactiveCursor.setPath(new Object[0]);
		}
		return unactiveCursor;
	}
	
	public BitCursor getBitLocation() {
		return bitLocation;
	}

	public void setBitLocation(BitCursor bitLocation) {
		this.bitLocation = bitLocation;
	}

	public List getPathList() {
		if (pathList == null) {
			pathList = new LinkedList();
			if (path != null) {
				Collections.addAll(pathList, path);
			}
		}
		
		return pathList;
	}
	
	public Object[] getPath() {
		if (pathList == null && path != null) {
			return path;
		}
		
		return getPathList().toArray();
	}

	public void setPath(Object[] path) {
		this.path = path;
		pathList = null;
	}	
	
	public LayoutCursorDescender descender() {
		return new LayoutCursorDescender(this);
	}
	
	public String toString() {
		String r = "LayoutCursor loc:" + bitLocation + " path(" + getPath().length + ")";
		for (Object o : getPath()) {
			r += " " + o;
		}
		return r;
	}
	
	public Rectangle getBlinkyLocation() {
		return blinkyLocation;
	}
	public void setBlinkyLocation(Rectangle blinkyLocation) {
		this.blinkyLocation = blinkyLocation;
	}

	
	/** @return a builder so this can be rebuilt */
	public LayoutCursorBuilder toBuilder() {
		LayoutCursorBuilder b = new LayoutCursorBuilder();
		b.setBitLocation(getBitLocation());
		b.setPath(getPath());
		return b;
	}

}
 