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
 * QNode, for Qubero Node.
 * A node that can tell you what its sources are, and how many sinks it's got (for now)
 * 
 * Now rolled into Resource
 *
 * @author Peter Halasz
 */


package net.pengo.resource;

public interface Resource_QNode
{
    //public InfoLink[] getInfoLinkList();
    //public DepLink[] getDepLinkList();
    
    public Resource[] getSources();
    
    // temporary interface. useful for telling if this qnode is "owned" by its sink, or independent, or multiply-used
    public int getSinkCount();
    public void addSink(Resource sink);
    public void removeSink(Resource sink);
    
    // listener stuff? etc etc
    
}


