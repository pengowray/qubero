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
 * ValuePage.java
 *
 * Created on July 11, 2003, 11:51 PM
 */

package net.pengo.propertyEditor;

import javax.swing.JLabel;
import javax.swing.JTextField;

import net.pengo.resource.Resource;
/**
 *
 * @author  Peter Halasz
 */
public class NamePage extends EditablePage {
    private Resource res;
    private JTextField inputField;
    
     public NamePage(Resource res) {
         this(res, null);
     }
     
    public NamePage(Resource res, PropertiesForm form) {
        super(form);
        this.res = res;
        this.form = form;

        add(new JLabel( "Name: " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
            
        add(inputField);
        build();
    }
    
    
    public void saveOp() {
        String name = inputField.getText();
        if (name==null || name.equals("")) {
            res.setName(null);
        } else {
            res.setName(name);
        }
    }
    
    public void buildOp() {
        String name = res.getName();
        if (name==null || name.equals("")) {
            inputField.setText("");
        } else {
            inputField.setText(name);
        }
    }
    
    public boolean isValid() {
        //fixme
        return true; // success
    }
    
    public String toString() {
        return "Name";
    }
}
