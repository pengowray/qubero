/*
 * Created on 3/02/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.hexdraw.layout;

import java.awt.Rectangle;
import java.util.Collections;
import java.util.LinkedList;
import java.util.List;

import net.pengo.bitSelection.BitCursor;

/**
 * @author Que
 *
 * TODO To change the template for this generated type comment go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
public class LayoutCursor {

	/* use	s pathList preferably, but if setPath() is used pathList will be lazily created */
	private Object[] path;
	private LinkedList pathList;
	
	private long x,y; // calculating cursor blink while initializing..
	
	private BitCursor bitLocation;
	private Rectangle blinkyLocation;
	private boolean blink = false;

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
		if (bitLocation == null)
			// FIXME: shouldn't do this. null valid. create an "addToBitLocation" method instead.
			bitLocation = new BitCursor();
				
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
	public long getX() {
		return x;
	}
	public void setX(long x) {
		this.x = x;
	}
	public long getY() {
		return y;
	}
	public void setY(long y) {
		this.y = y;
	}
}
 