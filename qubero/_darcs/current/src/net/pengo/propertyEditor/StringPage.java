/*
 * ValuePage.java
 *
 * Created on July 11, 2003, 11:51 PM
 */

package net.pengo.propertyEditor;

import javax.swing.JLabel;
import javax.swing.JTextField;

import net.pengo.resource.Resource;
import net.pengo.resource.StringResource;
/**
 *
 * @author  Peter Halasz
 */
public class StringPage extends EditablePage {
    private StringResource res;
    private JTextField inputField;
    
     public StringPage(StringResource res) {
         this(res, null);
     }
     
    public StringPage(StringResource res, PropertiesForm form) {
        super(form);
        this.res = res;
        this.form = form;

        add(new JLabel( "Value: " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
            
        add(inputField);
        build();
    }
    
    
    public void saveOp() {
    	//System.out.println("Text saveOp");
        String val = inputField.getText();
        res.setValue(val);
    }
    
    public void buildOp() {
        String val = res.getValue();
        if (val==null || val.equals("")) {
            inputField.setText("");
        } else {
            inputField.setText(val);
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
