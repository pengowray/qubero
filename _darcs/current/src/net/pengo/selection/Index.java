/**
 * Index.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.selection;

public class Index implements Comparable
{
    public long firstIndex;
    
    public Index(long index)
	{
		firstIndex = index;
    }
    
    public int compareTo(Object o)
	{
		Index other = (Index)o;
		return (this.firstIndex == other.firstIndex) ? 0 :((this.firstIndex < other.firstIndex) ? -1 : 1);
    }
}
