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

/*
 * IntResourceForm.java
 *
 * Created on July 9, 2003, 10:21 PM
 *
 * another attempt at IntInputBox
 */

package net.pengo.propertyEditor;

 
import javax.swing.JTree;
import javax.swing.tree.DefaultMutableTreeNode;

import net.pengo.resource.Resource;
import net.pengo.resource.ResourceAlertListener;

public class ResourceForm extends PropertiesForm implements ResourceAlertListener
{

    //private OpenFile openFile;
    private Resource res;
    
    public ResourceForm(Resource res) {
        super();
        this.res = res;
		
		//fixme: this should be a weak reference!
		res.addDeepAlertListener(this);
    }
    
    protected synchronized JTree getMenu() {
        DefaultMutableTreeNode root = deepSources(res);
        JTree jt = new JTree(root);
        jt.setSelectionPath(jt.getPathForRow(0));
        return jt;
    }
    
    public DefaultMutableTreeNode deepSources(Resource r) {
        
     	//PropertyPage[] pp = (PropertyPage[]) r.getPrimaryPages().toArray(new PropertyPage[0]);
    	//MultiPage mp = new MultiPage(this, pp, r.nameOrType());
        if (r==null) {
	    return new DefaultMutableTreeNode("[null]");
	}
	
        ResourceMultiPage mp = new ResourceMultiPage(this, r);
    	DefaultMutableTreeNode resNode = new DefaultMutableTreeNode(mp);
    	
    	// add pages as seperate nodes
    	// ...(dont bother.. we'll do them as a multipage)
//    	for (Iterator iter = pp.iterator(); iter.hasNext();) {
//    	    PropertyPage page = (PropertyPage) iter.next();
//    	    page.setForm(this);
//    	    DefaultMutableTreeNode dmtn = new DefaultMutableTreeNode(page);
//    	    resNode.add(dmtn);
//
//    	}
    	
    	Resource[] source = r.getSources();
    	for (int i = 0; i < source.length; i++) {
	        Resource resource = source[i];
	        //System.out.println("diving into:" + resource);
	        resNode.add(deepSources(resource));
	    }
	    
	    return resNode;
    }
	
	public void valueChanged(Resource updated)
	{
		//ResourceForm.this.rebuild();
		//System.out.println("rebuild of "+ ResourceForm.this.res + " form DUE TO " + updated);
		updateNav();
		
	}
	

	public void resourceRemoved(Resource removed)
	{
		updateNav();
	}
    
    
}
