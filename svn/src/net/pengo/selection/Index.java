/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * Index.java
 *
 * @author  Peter Halasz
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
