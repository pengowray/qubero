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
 * listEditor.java
 *
 * @author Peter Halasz
 */

package net.pengo.listEditor;

import net.pengo.propertyEditor.PropertiesForm;
import net.pengo.resource.ListResource;
import net.pengo.resource.Resource;

import javax.swing.JTree;
import javax.swing.tree.DefaultMutableTreeNode;
import java.util.Iterator;
import net.pengo.propertyEditor.ResourceMultiPage;

public class ListEditor extends PropertiesForm
{
	private ListResource list;
	
	public ListEditor(ListResource listResource) {
		super();
		
		this.list = listResource;
	}
	
	protected JTree getMenu()
	{
		
        DefaultMutableTreeNode root = new DefaultMutableTreeNode(new ListPage(this, list));
		
		for (Iterator i = list.iterator(); i.hasNext(); ) {
			Resource r = (Resource)i.next();
			root.add(deepSources(r));
		}
			
        JTree jt = new JTree(root);
        jt.setSelectionPath(jt.getPathForRow(0));
        return jt;
	}

	//* warning/fixme: taken directly from ResourceForm */
    public DefaultMutableTreeNode deepSources(Resource r) {
        
        ResourceMultiPage mp = new ResourceMultiPage(this, r);
    	DefaultMutableTreeNode resNode = new DefaultMutableTreeNode(mp);
    	
    	Resource[] source = r.getSources();
    	for (int i = 0; i < source.length; i++) {
	        Resource resource = source[i];
	        //System.out.println("diving into:" + resource);
	        resNode.add(deepSources(resource));
	    }
	    
	    return resNode;
    }

}

