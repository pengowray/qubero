/*
 * Created on 25/01/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.splash;

/**
 * @author Peter Halasz
 */
public class SimpleSize {
    private static final int defaultSimpleSize = 3;
    private static float sizes[] = {6, 7, 8, 11, 14, 24, 48, 64, 127, 256}; //FIXME: make this a list, allow user to add to.
    
    private int size;

    public SimpleSize() {
    	this.size = defaultSimpleSize;
    }

    public SimpleSize(int size) {
    	//FIXME: bounds check
    	this.size = size;
    }
    
    public int toInt() {
    	return size;
    }
    
    public float toFontSize() {
    	return sizes[size];
    }

	public static SimpleSize[] allSizes() {
		SimpleSize[] all = new SimpleSize[count()];
		for (int i=0; i<count(); i++) {
			all[i] = new SimpleSize(i);
		}
		
		return all;
	}
	
    
    /** valid simple sizes are 0 to simpleSizeCount()-1 */ 
    public static int count() {
    	return sizes.length;
    }
    
	public SimpleSize bigger() {
		if (size+1 < count())
			return new SimpleSize(size+1);
		
		return this; // if already max size
	}

	public SimpleSize smaller() {
		if (size > 0)
			return new SimpleSize(size-1);
		
		return this; // if already min size
	}
	
	public String toString() {
		return size +"";
	}
	
	public static SimpleSize defaultSimpleSize() {
		return new SimpleSize(defaultSimpleSize);		
	}
	
}
