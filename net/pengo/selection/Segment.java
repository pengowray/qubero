/**
 * Segment.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.selection;

public class Segment extends Index
{
    public long lastIndex;
    
    public Segment(long index0, long index1)
	{
		super(Math.min(index0, index1));
		lastIndex = Math.max(index0, index1);
    }
    
	public String toString() {
		return "Segment " + firstIndex + "-" + lastIndex;
	}
}
